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
from pathlib import Path
import signal
import time
import uuid

from .io import sha, write
from .transport import decode_row

FRAME_BYTES = 1024 * 1024
WIRE_SCHEMA = 'arkguardrl-core/live-worker-pipe/v4'


def _receive(connection, deadline):
    left = deadline-time.monotonic()
    if left <= 0 or not connection.poll(left):
        raise TimeoutError('Live worker response deadline exceeded')
    return connection.recv()


def _peak_rss():
    import resource
    import sys
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return int(value if sys.platform == 'darwin' else value*1024)


def _worker(connection, data_connection, ctx, options):
    if os.name == 'posix':
        os.setsid()
    from arkguardrl import learn
    from . import checkpoint, sampling
    from .routes.session import RouteSessionPool
    from .streaming import RowSender
    from .inference import pack_tensors, unpack_tensors
    learn._setup_torch(threads=1)
    pool = RouteSessionPool()
    model = anchor = None
    actor_key = anchor_key = None
    sender = None
    try:
        connection.send({'kind': 'ready', 'pid': os.getpid(),
                         'pgid': os.getpgrp() if os.name == 'posix' else None})
        while True:
            idle_started = time.monotonic()
            task = _receive(connection, ctx.deadline)
            idle_seconds = time.monotonic()-idle_started
            if task['kind'] == 'stop':
                break
            task_started = time.monotonic()
            token = task['token']
            ctx.check()
            keys = (sha(task['checkpoint']), sha(task['anchor']))
            if keys != (task['policy_sha256'], task['anchor_sha256']):
                raise ValueError('Worker checkpoint file identity changed')
            load_started = time.monotonic()
            actor_loads = anchor_loads = 0
            if not options['model_cache'] or actor_key != keys[0]:
                model, _ = checkpoint.load(task['checkpoint'], ctx.contract)
                actor_key, actor_loads = keys[0], 1
            if not options['model_cache'] or anchor_key != keys[1]:
                anchor, _ = checkpoint.load(task['anchor'], ctx.contract)
                anchor.requires_grad_(False)
                anchor_key, anchor_loads = keys[1], 1
            # Same contents may be saved at a new path; bind the current path by
            # updating only the path of a previously strictly verified receipt.
            for policy, path in ((model, task['checkpoint']), (anchor, task['anchor'])):
                receipt = policy._core_load_receipt
                policy._core_load_receipt = (str(Path(path).resolve()), *receipt[1:])
            load_seconds = time.monotonic()-load_started
            if sampling.weights_hash(model) != task['weights_sha256']:
                raise ValueError('Worker actor parameter identity mismatch')
            sender = RowSender(data_connection, token, ctx, options['transfer_window'], options['queue_rows'], options['block_rows'])
            inference_bytes = 0

            def infer(inputs, index):
                nonlocal inference_bytes
                packet, size = pack_tensors(inputs, ctx.config['max_episode_bytes'])
                inference_bytes += size
                connection.send(dict(kind='infer', token=token, index=index, packet=packet))
                reply = _receive(connection, ctx.deadline)
                if reply.get('kind') != 'prediction' or reply.get('token') != token or reply.get('index') != index:
                    raise ValueError('Inference reply identity/order mismatch')
                values = unpack_tensors(reply['packet'], {'logp', 'anchor'}, ctx.config['max_episode_bytes'])
                if (values['logp'].shape != (len(inputs['candidates']),)
                        or values['anchor'].shape != values['logp'].shape
                        or type(reply['value']) not in (int, float)):
                    raise ValueError('Inference reply shape/value mismatch')
                return values['logp'], reply['value'], values['anchor']

            starts, startup = pool.starts, pool.start_seconds
            rows, report = sampling.collect(ctx, model, anchor,
                seed=task['seed'], rng_seed=task['rng_seed'], revision=task['revision'],
                checkpoint=task['checkpoint'], directory=task['directory'],
                log_mode=task['log_mode'], session_pool=pool,
                on_row=sender.submit if options['transfer_mode'] == 'stream' else None,
                inference=infer if options['inference_mode'] != 'local' else None)
            if sha(task['checkpoint']) != keys[0] or sha(task['anchor']) != keys[1]:
                raise ValueError('Worker checkpoint changed during collection')
            report.update(worker_pid=os.getpid(), node_starts=pool.starts-starts,
                          node_start_seconds=pool.start_seconds-startup,
                          node_pid=pool.session._process.pid, load_seconds=load_seconds,
                          actor_loads=actor_loads, anchor_loads=anchor_loads,
                          worker_idle_seconds=idle_seconds, inference_request_bytes=inference_bytes)
            connection.send(dict(kind='episode', token=token, rows=len(rows), report=report, policy_sha256=keys[0]))
            transfer_started = time.monotonic()
            if options['transfer_mode'] == 'episode':
                for row in rows:
                    sender.submit(row)
            del rows
            stats = sender.finish()
            stats.update(transfer_seconds=time.monotonic()-transfer_started,
                         task_seconds=time.monotonic()-task_started, worker_peak_rss_bytes=_peak_rss())
            sender = None
            connection.send(dict(kind='done', token=token, **stats))
    except BaseException as exc:
        try:
            connection.send({'kind': 'error', 'error': repr(exc)})
        except (BrokenPipeError, EOFError, OSError):
            pass
    finally:
        if sender is not None:
            sender.abort()
        pool.close()
        data_connection.close()
        connection.close()


class Collector:
    """Reusable across PPO iterations; no learning or GPU model in the workers."""
    def __init__(self, ctx, workers=4, *, transfer_mode='stream', transfer_window=4, queue_rows=2,
                 model_cache=True, inference_mode='local', inference_wait_ms=2., block_rows=8):
        if type(workers) is not int or not 1 <= workers <= 16:
            raise ValueError('Collector workers must be 1..16')
        if transfer_mode not in ('stream', 'episode') or inference_mode not in ('local', 'batch-cpu', 'batch-mps'):
            raise ValueError('Unknown explicit collection execution mode')
        if (type(transfer_window) is not int or not 1 <= transfer_window <= 16
                or type(queue_rows) is not int or not 1 <= queue_rows <= 8
                or not 0 <= inference_wait_ms <= 50 or type(model_cache) is not bool
                or type(block_rows) is not int or not 1 <= block_rows <= 32):
            raise ValueError('Invalid bounded collector settings')
        self.options = dict(transfer_mode=transfer_mode, transfer_window=transfer_window,
                            queue_rows=queue_rows, block_rows=block_rows, model_cache=model_cache,
                            inference_mode=inference_mode, inference_wait_ms=inference_wait_ms)
        self._anchor = self._anchor_key = None
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
                data_parent, data_child = context.Pipe()
                process = context.Process(target=_worker, args=(child, data_child, self.ctx, self.options))
                process.start()
                child.close()
                data_child.close()
                self.children.append({'process': process, 'pipe': parent, 'data_pipe': data_parent, 'pgid': None})
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
        self.ctx.check(sources=True)
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
        inference = None
        inference_started = time.monotonic()
        if self.options['inference_mode'] != 'local':
            from . import checkpoint as checkpoints
            from .inference import BatchedInference
            if self._anchor_key != anchor_sha:
                self._anchor, _ = checkpoints.load(anchor, self.ctx.contract)
                self._anchor_key = anchor_sha
            inference = BatchedInference(model, self._anchor, self.options['inference_mode'][6:],
                                         self.ctx.ppo.execution_memory_bytes)
        inference_setup_seconds = time.monotonic()-inference_started
        requests = []
        first_request = None
        dispatch_finished = None
        worker_completed = {child['process'].pid: 0 for child in self.children}
        total_wire, total_capture, total_raw = 0, 0, 0

        def dispatch(child):
            nonlocal dispatch_finished
            if pending:
                task = pending.popleft()
                active[child['pipe']] = {'child': child, 'task': task, 'rows': [],
                    'report': None, 'buffer': bytearray(), 'row_length': None,
                    'digest': None, 'block_rows': None, 'wire_bytes': 0, 'raw_bytes': 0, 'decode_seconds': 0.0,
                    'next_inference': 0, 'inference_pending': False}
                child['pipe'].send(task)
                if not pending:
                    dispatch_finished = time.monotonic()

        for child in self.children:
            dispatch(child)
        try:
            while active:
                self.ctx.check()
                channels = {channel: state for control, state in active.items()
                            for channel in (control, state['child']['data_pipe'])}
                timeout = min(1.0, max(0, self.ctx.deadline-time.monotonic()))
                if requests:
                    timeout = min(timeout, max(0., self.options['inference_wait_ms']/1000-(time.monotonic()-first_request)))
                ready = wait(list(channels), timeout=timeout)
                for connection in ready:
                    state = channels[connection]
                    message = connection.recv()
                    task = state['task']
                    kind = message['kind']
                    if (kind == 'frame') != (connection is state['child']['data_pipe']):
                        raise ValueError('Message arrived on wrong collector channel')
                    if kind == 'error':
                        # A reported failure is already unwinding its Node pool.
                        # Allow that cleanup before escalating to group signals.
                        self.close()
                        raise RuntimeError('Live worker failed: '+message['error'])
                    if message.get('token') != task['token']:
                        raise ValueError('Worker task token mismatch')
                    if kind == 'infer':
                        from .inference import unpack_tensors, INPUTS
                        if (inference is None or state['inference_pending'] or state['report'] is not None
                                or message['index'] != state['next_inference']
                                or message['index'] >= self.ctx.config['max_decisions']):
                            raise ValueError('Unexpected or reordered inference request')
                        row = unpack_tensors(message['packet'], INPUTS, self.ctx.config['max_episode_bytes'])
                        state['inference_pending'] = True
                        requests.append((connection, state, message['index'], row))
                        if first_request is None:
                            first_request = time.monotonic()
                    elif kind == 'episode':
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
                        if (type(message.get('rows')) is not int or not 1 <= message['rows'] <= self.options['block_rows']
                                or len(state['rows'])+message['rows'] > self.ctx.config['max_decisions']
                                or message['index'] != len(state['rows'])
                                or len(state['rows']) >= self.ctx.config['max_decisions']
                                or (state['report'] is not None and len(state['rows'])+message['rows'] > state['report']['decisions'])
                                or not isinstance(part, bytes) or not 0 < len(part) <= FRAME_BYTES
                                or message['offset'] != len(state['buffer'])
                                or not 0 < message['length'] <= self.ctx.config['max_episode_bytes']):
                            raise ValueError('Live frame bounds/order mismatch')
                        if state['row_length'] is None:
                            state['row_length'], state['digest'] = message['length'], message['sha256']
                            state['block_rows'] = message['rows']
                        if (message['length'] != state['row_length'] or message['sha256'] != state['digest']
                                or message['rows'] != state['block_rows']):
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
                            if self.options['block_rows'] == 1:
                                row, raw_bytes = decode_row(state['buffer'], self.ctx.config['max_episode_bytes'])
                                block = [row]
                            else:
                                from .columnar import decode_rows
                                block, raw_bytes = decode_rows(state['buffer'], self.ctx.config['max_episode_bytes'], state['block_rows'])
                            state['decode_seconds'] += time.monotonic()-decode_started
                            state['raw_bytes'] += raw_bytes
                            total_raw += raw_bytes
                            if total_raw > self.ctx.ppo.max_buffer_bytes:
                                raise MemoryError('Inflated live transport exceeds buffer budget')
                            for row in block:
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
                            state['row_length'] = state['digest'] = state['block_rows'] = None
                        connection.send(('ack', task['token'], message['index'], message['offset']))
                    elif kind == 'done':
                        if (state['report'] is None or state['buffer']
                                or len(state['rows']) != state['report']['decisions']
                                or state['wire_bytes'] != message['wire_bytes']
                                or state['raw_bytes'] != message['raw_bytes']):
                            raise ValueError('Incomplete live episode transfer')
                        if state['inference_pending']:
                            raise ValueError('Episode ended with pending inference')
                        state['report'].update({k: v for k, v in message.items() if k not in ('kind', 'token')})
                        state['report']['decode_seconds'] = state['decode_seconds']
                        worker_completed[state['child']['process'].pid] += 1
                        results[task['index']] = (state['rows'], state['report'])
                        del active[state['child']['pipe']]
                        dispatch(state['child'])
                    else:
                        raise ValueError('Unknown live worker message')
                if requests and (len(requests) >= len(active) or
                        time.monotonic()-first_request >= self.options['inference_wait_ms']/1000):
                    from .inference import pack_tensors
                    predictions = inference.evaluate([request[3] for request in requests])
                    for (connection, state, index, _), (logp, value, anchor_logp) in zip(requests, predictions):
                        packet, _ = pack_tensors({'logp': logp, 'anchor': anchor_logp}, self.ctx.config['max_episode_bytes'])
                        connection.send(dict(kind='prediction', token=state['task']['token'], index=index,
                                             packet=packet, value=value))
                        state['next_inference'] += 1
                        state['inference_pending'] = False
                    requests.clear()
                    first_request = None
            if sha(checkpoint) != policy_sha or sha(anchor) != anchor_sha or weights_hash(model) != parameters:
                raise ValueError('Parent policy/anchor changed during collection')
            rows, reports = [], []
            for index in range(len(seeds)):
                episode_rows, report = results.pop(index)
                rows.extend(episode_rows)
                reports.append(report)
            write(self.ctx.root/f'episodes-r{revision:04d}.json', reports)
            write(self.ctx.root/'episodes.json', reports)
            timing = {'schema': WIRE_SCHEMA, 'workers': self.workers,
                'collection_seconds': time.monotonic()-started,
                'worker_startup_seconds': self.startup_seconds, 'wire_bytes': total_wire,
                'raw_transport_bytes': total_raw,
                'capture_bytes_upper_bound': total_capture,
                'node_starts': sum(r['node_starts'] for r in reports),
                'decisions_per_second': len(rows)/(time.monotonic()-started),
                'worker_completed_games': worker_completed,
                'last_dispatch_to_finish_seconds': time.monotonic()-dispatch_finished,
                'task_occupancy': sum(r['task_seconds'] for r in reports)/(self.workers*(time.monotonic()-started)),
                'parent_peak_rss_bytes': _peak_rss(), 'collector_options': self.options,
                'inference_setup_seconds': inference_setup_seconds,
                'inference': inference.report() if inference is not None else None}
            self.ctx.check(sources=True)
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
            child['data_pipe'].close()
            if process.is_alive():
                raise RuntimeError('Collector worker did not exit')
            process.close()
        self.children = []

    def __exit__(self, exc_type, *_):
        self.close(force=exc_type is not None)
