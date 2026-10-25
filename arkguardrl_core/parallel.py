"""Persistent spawn collectors with bounded, acknowledged live-row transport.

Workers own CPU policies and native processes. The parent learns only after all
requested natural episodes have arrived, ordered by task and decision index.
Pickle is used solely over private pipes to children created by this collector;
there is no artifact or historical-row loading entrypoint.
"""
from collections import deque
import hashlib
import multiprocessing as mp
from multiprocessing.connection import wait
import os
import signal
import time
import uuid

from .io import sha, write
from .transport import encode_row, decode_row

FRAME_BYTES = 1024 * 1024
WIRE_SCHEMA = 'arkguardrl-core/live-worker-pipe/v2'


def _receive(connection, deadline):
    left = deadline-time.monotonic()
    if left <= 0 or not connection.poll(left):
        raise TimeoutError('Live worker response deadline exceeded')
    return connection.recv()


def _worker(connection, ctx):
    # A process group includes this worker and its Node child, never other jobs.
    if os.name == 'posix':
        os.setsid()
    from arkguardrl import learn
    from . import checkpoint, sampling
    from .routes.session import RouteSessionPool
    learn._setup_torch(threads=1)
    pool = RouteSessionPool()
    model = anchor = None
    loaded = None
    try:
        connection.send({'kind': 'ready', 'pid': os.getpid(),
                         'pgid': os.getpgrp() if os.name == 'posix' else None})
        while True:
            task = _receive(connection, ctx.deadline)
            if task['kind'] == 'stop':
                break
            token = task['token']
            ctx.check()
            keys = (sha(task['checkpoint']), sha(task['anchor']))
            if keys != (task['policy_sha256'], task['anchor_sha256']):
                raise ValueError('Worker checkpoint file identity changed')
            if keys != loaded:
                model, _ = checkpoint.load(task['checkpoint'], ctx.contract)
                anchor, _ = checkpoint.load(task['anchor'], ctx.contract)
                anchor.requires_grad_(False)
                loaded = keys
            if sampling.weights_hash(model) != task['weights_sha256']:
                raise ValueError('Worker actor parameter identity mismatch')
            starts, startup = pool.starts, pool.start_seconds
            rows, report = sampling.collect(ctx, model, anchor,
                seed=task['seed'], rng_seed=task['rng_seed'], revision=task['revision'],
                checkpoint=task['checkpoint'], directory=task['directory'],
                log_mode=task['log_mode'], session_pool=pool)
            if sha(task['checkpoint']) != keys[0] or sha(task['anchor']) != keys[1]:
                raise ValueError('Worker checkpoint changed during collection')
            report.update(worker_pid=os.getpid(), node_starts=pool.starts-starts,
                          node_start_seconds=pool.start_seconds-startup,
                          node_pid=pool.session._process.pid)
            transfer_started = time.monotonic()
            connection.send({'kind': 'episode', 'token': token, 'rows': len(rows),
                             'report': report, 'policy_sha256': keys[0]})
            pending = deque(rows)
            del rows
            wire_bytes = 0
            raw_bytes = tensor_bytes = metadata_bytes = 0
            encode_seconds = 0.0
            for index in range(report['decisions']):
                row = pending.popleft()
                encode_started = time.monotonic()
                payload, sizes = encode_row(row, ctx.config['max_episode_bytes'])
                encode_seconds += time.monotonic()-encode_started
                raw_bytes += sizes['raw_bytes']
                tensor_bytes += sizes['tensor_bytes']
                metadata_bytes += sizes['metadata_bytes']
                del row
                if len(payload) > ctx.config['max_episode_bytes']:
                    raise MemoryError('Single live row exceeds transport bound')
                wire_bytes += len(payload)
                if wire_bytes > ctx.ppo.max_buffer_bytes or raw_bytes > ctx.ppo.max_buffer_bytes:
                    raise MemoryError('Worker live transport exceeds buffer bound')
                digest = hashlib.sha256(payload).hexdigest()
                for offset in range(0, len(payload), FRAME_BYTES):
                    connection.send({'kind': 'frame', 'token': token, 'index': index,
                        'length': len(payload), 'offset': offset, 'sha256': digest,
                        'payload': payload[offset:offset+FRAME_BYTES]})
                    ack = _receive(connection, ctx.deadline)
                    if ack != ('ack', token, index, offset):
                        raise ValueError('Live frame acknowledgement mismatch')
                del payload
            connection.send({'kind': 'done', 'token': token, 'wire_bytes': wire_bytes,
                             'raw_bytes': raw_bytes, 'tensor_bytes': tensor_bytes,
                             'metadata_bytes': metadata_bytes, 'encode_seconds': encode_seconds,
                             'transfer_seconds': time.monotonic()-transfer_started})
    except BaseException as exc:
        try:
            connection.send({'kind': 'error', 'error': repr(exc)})
        except (BrokenPipeError, EOFError, OSError):
            pass
    finally:
        pool.close()
        connection.close()


class Collector:
    """Reusable across PPO iterations; no learning or GPU model in the workers."""
    def __init__(self, ctx, workers=4):
        if type(workers) is not int or not 1 <= workers <= 16:
            raise ValueError('Collector workers must be 1..16')
        self.ctx, self.workers = ctx, workers
        self.children = []
        self.startup_seconds = 0.0

    def __enter__(self):
        started = time.monotonic()
        context = mp.get_context('spawn')
        # spawn imports numerical libraries before _worker runs. Prevent excess
        # BLAS/OpenMP threads there as well as setting Torch threads in _worker.
        names = ('OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'MKL_NUM_THREADS',
                 'VECLIB_MAXIMUM_THREADS', 'NUMEXPR_NUM_THREADS')
        previous = {name: os.environ.get(name) for name in names}
        try:
            for name in names:
                os.environ[name] = '1'
            for _ in range(self.workers):
                parent, child = context.Pipe()
                process = context.Process(target=_worker, args=(child, self.ctx))
                process.start()
                child.close()
                self.children.append({'process': process, 'pipe': parent, 'pgid': None})
            for child in self.children:
                ready = _receive(child['pipe'], self.ctx.deadline)
                if (ready.get('kind') != 'ready' or ready['pid'] != child['process'].pid
                        or (os.name == 'posix' and ready['pgid'] != ready['pid'])):
                    raise RuntimeError('Worker process-group handshake failed')
                child['pgid'] = ready['pgid']
        except BaseException:
            self.close(force=True)
            raise
        finally:
            for name, value in previous.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value
        self.startup_seconds = time.monotonic()-started
        return self

    def collect(self, model, *, checkpoint, anchor, seeds, rng_seed, revision=0):
        from .sampling import weights_hash
        if not self.children:
            raise RuntimeError('Use Collector as a context manager')
        if not seeds or len(set(seeds)) != len(seeds) or not all(type(s) is int and 0 < s <= 0xffffffff for s in seeds):
            raise ValueError('Unique uint32 game seeds required')
        if type(rng_seed) is not int or not 0 <= rng_seed <= 0xffffffff-len(seeds):
            raise ValueError('Sampling RNG range outside uint32')
        self.ctx.check()
        started = time.monotonic()
        policy_sha, anchor_sha = sha(checkpoint), sha(anchor)
        parameters = weights_hash(model)
        pending = deque()
        for index, seed in enumerate(seeds):
            log_mode = ('full' if self.ctx.collection_index % self.ctx.config['full_trace_every'] == 0
                        else 'compact')
            self.ctx.collection_index += 1
            pending.append({'kind': 'collect', 'token': uuid.uuid4().hex, 'index': index,
                'seed': seed, 'rng_seed': rng_seed+index, 'revision': revision,
                'checkpoint': str(checkpoint), 'anchor': str(anchor),
                'policy_sha256': policy_sha, 'anchor_sha256': anchor_sha,
                'weights_sha256': parameters, 'log_mode': log_mode,
                'directory': str(self.ctx.root/'games'/f'r{revision:04d}-{index:03d}-{seed}')})
        active, results = {}, {}
        total_wire, total_capture, total_raw = 0, 0, 0

        def dispatch(child):
            if pending:
                task = pending.popleft()
                active[child['pipe']] = {'child': child, 'task': task, 'rows': [],
                    'report': None, 'buffer': bytearray(), 'row_length': None,
                    'digest': None, 'wire_bytes': 0, 'raw_bytes': 0, 'decode_seconds': 0.0}
                child['pipe'].send(task)

        for child in self.children:
            dispatch(child)
        try:
            while active:
                self.ctx.check()
                ready = wait(list(active), timeout=min(1.0, max(0, self.ctx.deadline-time.monotonic())))
                for connection in ready:
                    state = active[connection]
                    message = connection.recv()
                    task = state['task']
                    kind = message['kind']
                    if kind == 'error':
                        raise RuntimeError('Live worker failed: '+message['error'])
                    if message.get('token') != task['token']:
                        raise ValueError('Worker task token mismatch')
                    if kind == 'episode':
                        report = message['report']
                        if (state['report'] is not None or message['policy_sha256'] != policy_sha
                                or report['policy_sha256'] != policy_sha or report['weights_sha256'] != parameters
                                or report['policy_revision'] != revision or report['seed'] != task['seed']
                                or report['rng_seed'] != task['rng_seed'] or report['status'] != 'personal-ended'
                                or report['decisions'] != message['rows'] or not 1 <= message['rows'] <= self.ctx.config['max_decisions']
                                or report['trajectory_audit']['status'] not in ('personal-ended', 'game-ended')
                                or not report['trajectory_audit']['public_feedback_verified']):
                            raise ValueError('Incomplete/mismatched worker episode')
                        total_capture += report['capture_bytes_upper_bound']
                        if total_capture > self.ctx.ppo.max_buffer_bytes:
                            raise MemoryError('Combined live captures exceed buffer budget')
                        state['report'] = report
                    elif kind == 'frame':
                        part = message['payload']
                        if (state['report'] is None or message['index'] != len(state['rows'])
                                or len(state['rows']) >= state['report']['decisions']
                                or not isinstance(part, bytes) or not 0 < len(part) <= FRAME_BYTES
                                or message['offset'] != len(state['buffer'])
                                or not 0 < message['length'] <= self.ctx.config['max_episode_bytes']):
                            raise ValueError('Live frame bounds/order mismatch')
                        if state['row_length'] is None:
                            state['row_length'], state['digest'] = message['length'], message['sha256']
                        if message['length'] != state['row_length'] or message['sha256'] != state['digest']:
                            raise ValueError('Live row frame identity changed')
                        state['buffer'].extend(part)
                        total_wire += len(part)
                        state['wire_bytes'] += len(part)
                        if len(state['buffer']) > message['length'] or total_wire > self.ctx.ppo.max_buffer_bytes:
                            raise MemoryError('Live transport exceeds buffer budget')
                        if len(state['buffer']) == message['length']:
                            if hashlib.sha256(state['buffer']).hexdigest() != state['digest']:
                                raise ValueError('Live row transport digest mismatch')
                            decode_started = time.monotonic()
                            row, raw_bytes = decode_row(state['buffer'], self.ctx.config['max_episode_bytes'])
                            state['decode_seconds'] += time.monotonic()-decode_started
                            state['raw_bytes'] += raw_bytes
                            total_raw += raw_bytes
                            if total_raw > self.ctx.ppo.max_buffer_bytes:
                                raise MemoryError('Inflated live transport exceeds buffer budget')
                            transition = row.transition
                            expected_episode = ('games/'
                                f'{os.path.basename(task["directory"])}/{task["seed"]}/{revision}')
                            if (transition is None or transition.episode_id != expected_episode
                                    or transition.decision_index != len(state['rows'])
                                    or row.capture['episode_id'] != expected_episode
                                    or row.policy_sha != policy_sha
                                    or any(v.device.type != 'cpu' for v in row.inputs().values())):
                                raise ValueError('Live row identity/device mismatch')
                            state['rows'].append(row)
                            state['buffer'] = bytearray()
                            state['row_length'] = state['digest'] = None
                        connection.send(('ack', task['token'], message['index'], message['offset']))
                    elif kind == 'done':
                        if (state['report'] is None or state['buffer']
                                or len(state['rows']) != state['report']['decisions']
                                or state['wire_bytes'] != message['wire_bytes']
                                or state['raw_bytes'] != message['raw_bytes']):
                            raise ValueError('Incomplete live episode transfer')
                        state['report'].update(transfer_seconds=message['transfer_seconds'],
                            wire_bytes=message['wire_bytes'], raw_bytes=message['raw_bytes'],
                            tensor_bytes=message['tensor_bytes'], metadata_bytes=message['metadata_bytes'],
                            encode_seconds=message['encode_seconds'], decode_seconds=state['decode_seconds'])
                        results[task['index']] = (state['rows'], state['report'])
                        del active[connection]
                        dispatch(state['child'])
                    else:
                        raise ValueError('Unknown live worker message')
            if sha(checkpoint) != policy_sha or sha(anchor) != anchor_sha or weights_hash(model) != parameters:
                raise ValueError('Parent policy/anchor changed during collection')
            rows, reports = [], []
            for index in range(len(seeds)):
                episode_rows, report = results.pop(index)
                rows.extend(episode_rows)
                reports.append(report)
            write(self.ctx.root/'episodes.json', reports)
            timing = {'schema': WIRE_SCHEMA, 'workers': self.workers,
                'collection_seconds': time.monotonic()-started,
                'worker_startup_seconds': self.startup_seconds, 'wire_bytes': total_wire,
                'raw_transport_bytes': total_raw,
                'capture_bytes_upper_bound': total_capture,
                'node_starts': sum(r['node_starts'] for r in reports)}
            return rows, reports, timing
        except BaseException:
            self.close(force=True)
            raise

    def close(self, force=False):
        for child in self.children:
            process = child['process']
            if not force and process.is_alive():
                try:
                    child['pipe'].send({'kind': 'stop'})
                except (BrokenPipeError, EOFError, OSError):
                    pass
        for child in self.children:
            process = child['process']
            if not force:
                process.join(timeout=3)
            if force or process.is_alive():
                pgid = child['pgid']
                if os.name == 'posix' and pgid is not None:
                    try:
                        os.killpg(pgid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                elif process.is_alive():
                    process.terminate()
                process.join(timeout=2)
                if os.name == 'posix' and pgid is not None:
                    # Also catch a Node child left behind if Python exited first.
                    try:
                        os.killpg(pgid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                elif process.is_alive():
                    process.kill()
                process.join(timeout=2)
            child['pipe'].close()
            if process.is_alive():
                raise RuntimeError('Collector worker did not exit')
            process.close()
        self.children = []

    def __exit__(self, exc_type, *_):
        self.close(force=exc_type is not None)
