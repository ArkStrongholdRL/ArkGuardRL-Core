"""Persistent spawn collectors with bounded, acknowledged live-row transport.

Workers own CPU policies and native processes. The parent learns only after all
requested natural episodes have arrived, ordered by task and decision index.
Pickle is used solely over private pipes to children created by this collector;
there is no artifact or historical-row loading entrypoint.
"""
from collections import deque
from concurrent.futures import ThreadPoolExecutor
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
WIRE_SCHEMA = 'arkguardrl-core/live-worker-pipe/v12'


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


def _worker(connections, ctx, options):
    if os.name == 'posix':
        os.setsid()
    from contextlib import suppress
    from arkguardrl import learn
    from . import checkpoint, sampling
    from .routes.session import RouteSessionPool
    from .streaming import RowSender
    from .inference import pack_tensors, unpack_tensors
    learn._setup_torch(threads=1)
    model = anchor = None
    actor_key = anchor_key = None
    slots = {control: dict(connection=control, data=data, slot=i, pool=RouteSessionPool(),
                           episode=None, sender=None, idle_since=time.monotonic())
             for i, (control, data) in enumerate(connections)}
    current = connections[0][0]

    def advance(state, prediction=None):
        connection, pool = state['connection'], state['pool']
        task = state['task']
        try:
            inputs, index, selector = state['episode'].send(prediction)
        except StopIteration as complete:
            rows, report = complete.value
            state['episode'] = None
            if (sha(task['checkpoint']), sha(task['anchor'])) != state['keys']:
                raise ValueError('Worker checkpoint changed during collection')
            report.update(worker_pid=os.getpid(), worker_slot=state['slot'],
                node_starts=pool.starts-state['starts'], node_start_seconds=pool.start_seconds-state['startup'],
                node_pid=pool.session._process.pid, load_seconds=state['load_seconds'],
                actor_loads=state['actor_loads'], anchor_loads=state['anchor_loads'],
                worker_idle_seconds=state['idle_seconds'], inference_request_bytes=state['inference_bytes'])
            connection.send(dict(kind='episode', token=task['token'], rows=len(rows), report=report,
                                 policy_sha256=state['keys'][0]))
            transfer_started = time.monotonic()
            if options['transfer_mode'] == 'episode':
                for row in rows:
                    state['sender'].submit(row)
            del rows, complete
            stats = state['sender'].finish()
            stats.update(transfer_seconds=time.monotonic()-transfer_started,
                         task_seconds=time.monotonic()-state['task_started'], worker_peak_rss_bytes=_peak_rss())
            state['sender'] = None
            state['idle_since'] = time.monotonic()
            connection.send(dict(kind='done', token=task['token'], **stats))
            return
        packet, size = pack_tensors(inputs, ctx.config['max_episode_bytes'])
        state['inference_bytes'] += size
        state['request'] = (index, selector, len(inputs['menu'] if selector else inputs['candidates']))
        connection.send(dict(kind='infer', token=task['token'], index=index, packet=packet, selector=selector))

    try:
        for control, state in slots.items():
            control.send({'kind': 'ready', 'pid': os.getpid(), 'slot': state['slot'],
                          'pgid': os.getpgrp() if os.name == 'posix' else None})
        while slots:
            left = ctx.deadline-time.monotonic()
            if left <= 0:
                raise TimeoutError('Live worker response deadline exceeded')
            ready = wait(list(slots), timeout=left)
            for current in ready:
                state = slots[current]
                message = current.recv()
                if message['kind'] == 'stop':
                    if state['episode'] is not None:
                        raise RuntimeError('Cannot stop a slot with an active episode')
                    state['pool'].close()
                    state['data'].close()
                    current.close()
                    del slots[current]
                    continue
                if message['kind'] == 'prediction':
                    index, selector, count = state['request']
                    if (state['episode'] is None or message.get('token') != state['task']['token']
                            or message.get('index') != index):
                        raise ValueError('Inference reply identity/order mismatch')
                    values = unpack_tensors(message['packet'], {'logp'} if selector else {'logp', 'anchor'},
                                            ctx.config['max_episode_bytes'])
                    if (values['logp'].shape != (count,)
                            or (not selector and values['anchor'].shape != values['logp'].shape)
                            or type(message['value']) not in (int, float)):
                        raise ValueError('Inference reply shape/value mismatch')
                    advance(state, (values['logp'], message['value'], values.get('anchor')))
                    continue
                if message['kind'] != 'collect' or state['episode'] is not None:
                    raise ValueError('Unexpected worker slot task')
                task = message
                ctx.check()
                started = time.monotonic()
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
                for policy, path in ((model, task['checkpoint']), (anchor, task['anchor'])):
                    receipt = policy._core_load_receipt
                    policy._core_load_receipt = (str(Path(path).resolve()), *receipt[1:])
                load_seconds = time.monotonic()-load_started
                if sampling.weights_hash(model) != task['weights_sha256']:
                    raise ValueError('Worker actor parameter identity mismatch')
                sender = RowSender(state['data'], task['token'], ctx, options['transfer_window'],
                                   options['queue_rows'], options['block_rows'])
                state.update(task=task, task_started=started, keys=keys, sender=sender,
                    idle_seconds=started-state['idle_since'], load_seconds=load_seconds,
                    actor_loads=actor_loads, anchor_loads=anchor_loads, inference_bytes=0,
                    starts=state['pool'].starts, startup=state['pool'].start_seconds)
                state['episode'] = sampling.collect_steps(ctx, model, anchor,
                    seed=task['seed'], rng_seed=task['rng_seed'], revision=task['revision'],
                    checkpoint=task['checkpoint'], directory=task['directory'], log_mode=task['log_mode'],
                    session_pool=state['pool'], on_row=sender.submit if options['transfer_mode'] == 'stream' else None,
                    remote_inference=options['inference_mode'] != 'local')
                advance(state)
    except BaseException as exc:
        with suppress(BrokenPipeError, EOFError, OSError):
            current.send({'kind': 'error', 'error': repr(exc)})
    finally:
        for state in slots.values():
            if state['episode'] is not None:
                with suppress(Exception):
                    state['episode'].close()
            if state['sender'] is not None:
                state['sender'].abort()
            with suppress(Exception):
                state['pool'].close()
            state['data'].close()
            state['connection'].close()


class Collector:
    """Reusable across PPO iterations; no learning or GPU model in the workers."""
    def __init__(self, ctx, workers=4, *, transfer_mode='stream', transfer_window=4, queue_rows=2,
                 model_cache=True, inference_mode='local', inference_wait_ms=2., block_rows=8,
                 envs_per_worker=1):
        if type(workers) is not int or not 1 <= workers <= 16:
            raise ValueError('Collector workers must be 1..16')
        if transfer_mode not in ('stream', 'episode') or inference_mode not in ('local', 'batch-cpu', 'batch-mps', 'batch-cuda'):
            raise ValueError('Unknown explicit collection execution mode')
        if type(envs_per_worker) is not int or not 1 <= envs_per_worker <= 8:
            raise ValueError('Environments per worker must be 1..8')
        if envs_per_worker > 1 and inference_mode == 'local':
            raise ValueError('Multiple environments per worker require batched inference')
        if (type(transfer_window) is not int or not 1 <= transfer_window <= 16
                or type(queue_rows) is not int or not 1 <= queue_rows <= 8
                or not 0 <= inference_wait_ms <= 50 or type(model_cache) is not bool
                or type(block_rows) is not int or not 1 <= block_rows <= 32):
            raise ValueError('Invalid bounded collector settings')
        self.options = dict(transfer_mode=transfer_mode, transfer_window=transfer_window,
                            queue_rows=queue_rows, block_rows=block_rows, model_cache=model_cache,
                            inference_mode=inference_mode, inference_wait_ms=inference_wait_ms,
                            envs_per_worker=envs_per_worker)
        self._anchor = self._anchor_key = None
        self._inference = self._inference_architecture = None
        self._inference_runner = None
        self._reply_executor = None
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
                pipes = [(context.Pipe(), context.Pipe()) for _ in range(self.options['envs_per_worker'])]
                process = context.Process(target=_worker,
                    args=([(control[1], data[1]) for control, data in pipes], self.ctx, self.options))
                process.start()
                for slot, ((parent, child), (data_parent, data_child)) in enumerate(pipes):
                    child.close()
                    data_child.close()
                    self.children.append({'process': process, 'pipe': parent, 'data_pipe': data_parent,
                                          'pgid': None, 'slot': slot})
            for child in self.children:
                ready = _receive(child['pipe'], self.ctx.deadline)
                if (ready.get('kind') != 'ready' or ready['pid'] != child['process'].pid or ready['slot'] != child['slot']
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
        if self.options['envs_per_worker'] > 1:
            self._reply_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix='prediction-replies')
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
            log_mode = self.ctx.config['log_mode']
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
            from .inference import BatchedInference, InferenceExecutor
            if self._anchor_key != anchor_sha:
                self._anchor, _ = checkpoints.load(anchor, self.ctx.contract)
                self._anchor_key = anchor_sha
            architecture = (type(model), model.width, repr(getattr(model, 'planner_config', None)),
                            repr(getattr(model, 'specialist_config', None)))
            keys = dict(actor_key=parameters, anchor_key=self._anchor._core_load_receipt[-1])
            if self._inference is None or self._inference_architecture != architecture:
                if self._inference_runner is not None:
                    self._inference_runner.close()
                self._inference = BatchedInference(model, self._anchor, self.options['inference_mode'][6:],
                                                  self.ctx.ppo.execution_memory_bytes, **keys)
                self._inference_architecture = architecture
                self._inference_runner = InferenceExecutor(self._inference)
            else:
                self._inference.update(model, self._anchor, **keys)
            inference = self._inference
        inference_setup_seconds = time.monotonic()-inference_started
        runner = self._inference_runner if inference is not None else None
        requests = []
        replies = deque()
        queue_seconds, queued_states, max_queue = 0., 0, 0
        last_data_fd = -1
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
                while replies and replies[0].done():
                    replies.popleft().result()
                channels = {channel: state for control, state in active.items()
                            for channel in (control, state['child']['data_pipe'])}
                timeout = min(1.0, max(0, self.ctx.deadline-time.monotonic()))
                if requests and runner.batch is None:
                    timeout = min(timeout, max(0., self.options['inference_wait_ms']/1000-(time.monotonic()-requests[0][5])))
                ready = wait(list(channels)+([runner.ready] if runner is not None else []), timeout=timeout)
                if runner is not None and runner.ready in ready:
                    from .inference import pack_tensors
                    batch, predictions = runner.receive()
                    for (connection, state, index, _, selector, _), (logp, value, anchor_logp) in zip(batch, predictions):
                        fields = {'logp': logp} if selector else {'logp': logp, 'anchor': anchor_logp}
                        packet, _ = pack_tensors(fields, self.ctx.config['max_episode_bytes'])
                        message = dict(kind='prediction', token=state['task']['token'], index=index,
                                       packet=packet, value=value)
                        # A multi-env worker may be sending another slot's large
                        # request. Keep reading while its prediction pipe fills;
                        # blocking both pipe ends here would deadlock the worker.
                        if self._reply_executor is not None:
                            replies.append(self._reply_executor.submit(connection.send, message))
                        else:
                            connection.send(message)
                        state['next_inference'] += int(not selector)
                        state['inference_pending'] = False
                    ready.remove(runner.ready)
                # Recheck inference between bulk frames, rather than decoding
                # every ready worker's block before responding. Rotate data
                # pipes so large transfers cannot starve other workers.
                if runner is not None:
                    control_ready = [c for c in ready if c in active]
                    data_ready = sorted((c for c in ready if c not in active),
                                        key=lambda c: (c.fileno() <= last_data_fd, c.fileno()))
                    ready = control_ready+data_ready[:1]
                    if data_ready:
                        last_data_fd = data_ready[0].fileno()
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
                        expected_inputs = ({'reward_binding'} if 'binding' in getattr(model,'planner_config',{}) else set()) | ({'goal_relations'} if 'relations' in getattr(model,'planner_config',{}) or getattr(model,'planner_config',{}).get('options',{}).get('schema')=='arkguardrl-core/search-and-adapt-options/v1' else set()) | ({'option_context'} if 'options' in getattr(model,'planner_config',{}) else set()) | ({'recovery_context'} if 'recovery' in getattr(model,'planner_config',{}) else set()) | ({'plan_context'} if hasattr(model,'planner_config') else set()) | INPUTS | ({'prep_history'} if getattr(model, 'specialist_config', {}).get('prep_history') else set())
                        selector = message['selector']
                        row = unpack_tensors(message['packet'], {'state', 'menu'} if selector else expected_inputs,
                                             self.ctx.config['max_episode_bytes'])
                        state['inference_pending'] = True
                        requests.append((connection, state, message['index'], row, selector, time.monotonic()))
                        max_queue = max(max_queue, len(requests))
                    elif kind == 'episode':
                        report = message['report']
                        if (state['report'] is not None or message['policy_sha256'] != policy_sha
                                or report['policy_sha256'] != policy_sha or report['weights_sha256'] != parameters
                                or report['policy_revision'] != revision or report['seed'] != task['seed']
                                or report['rng_seed'] != task['rng_seed'] or report['status'] != 'personal-ended'
                                or report['decisions'] != message['rows'] or not 1 <= message['rows'] <= self.ctx.config['max_decisions']
                                or report['log_mode'] != task['log_mode']):
                            raise ValueError('Incomplete/mismatched worker episode')
                        if task['log_mode'] == 'full' and (
                                report['trajectory_audit']['status'] not in ('personal-ended', 'game-ended')
                                or not report['trajectory_audit']['public_feedback_verified']):
                            raise ValueError('Incomplete debug trajectory')
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
                if requests and runner.batch is None and (
                        len(requests) >= sum(s['report'] is None for s in active.values()) or
                        time.monotonic()-requests[0][5] >= self.options['inference_wait_ms']/1000):
                    # A selector unlocks its worker's next native input. Serve
                    # these short jobs first to let native requests accumulate.
                    # Each env slot must finish a native decision before submitting
                    # another selector, so this cannot starve native inference.
                    selector = any(r[4] for r in requests)
                    batch = [r for r in requests if r[4] == selector]
                    requests = [r for r in requests if r[4] != selector]
                    now = time.monotonic()
                    queue_seconds += sum(now-r[5] for r in batch)
                    queued_states += len(batch)
                    runner.submit(batch, selector)
            for reply in replies:
                reply.result()
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
                'envs_per_worker': self.options['envs_per_worker'],
                'collection_seconds': time.monotonic()-started,
                'worker_startup_seconds': self.startup_seconds, 'wire_bytes': total_wire,
                'raw_transport_bytes': total_raw,
                'capture_bytes_upper_bound': total_capture,
                'node_starts': sum(r['node_starts'] for r in reports),
                'decisions_per_second': len(rows)/(time.monotonic()-started),
                'worker_completed_games': worker_completed,
                'last_dispatch_to_finish_seconds': time.monotonic()-dispatch_finished,
                'task_occupancy': sum(r['task_seconds'] for r in reports)/(len(self.children)*(time.monotonic()-started)),
                'parent_peak_rss_bytes': _peak_rss(), 'collector_options': self.options,
                'inference_setup_seconds': inference_setup_seconds,
                'inference_queue': {'max_pending': max_queue,
                    'mean_wait_seconds': queue_seconds/max(1, queued_states)},
                'inference': inference.report() if inference is not None else None}
            self.ctx.check()
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
        for child in (c for c in self.children if c['slot'] == 0):
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
                    except PermissionError:
                        if process.is_alive():
                            raise
                elif process.is_alive():
                    process.terminate()
                process.join(timeout=2)
                if os.name == 'posix' and pgid is not None:
                    # Also catch a Node child left behind if Python exited first.
                    try:
                        os.killpg(pgid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    except PermissionError:
                        # macOS may reject signaling an exited process group.
                        # A still-live owned worker must remain a hard failure.
                        if process.is_alive():
                            raise
                elif process.is_alive():
                    process.kill()
                process.join(timeout=2)
            if process.is_alive():
                raise RuntimeError('Collector worker did not exit')
            process.close()
        if self._reply_executor is not None:
            self._reply_executor.shutdown(wait=True, cancel_futures=True)
            self._reply_executor = None
        for child in self.children:
            child['pipe'].close()
            child['data_pipe'].close()
        self.children = []
        if self._inference_runner is not None:
            self._inference_runner.close()
            self._inference_runner = None
        self._inference = self._inference_architecture = None

    def __exit__(self, exc_type, *_):
        self.close(force=exc_type is not None)
