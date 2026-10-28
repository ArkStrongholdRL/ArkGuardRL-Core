"""Live environment with optional full debug traces; recording is off by default."""
import hashlib
import gzip
from pathlib import Path

from arkguardrl.contracts import canonical
from arkguardrl.environment import ENV_SCHEMA
from arkguardrl.trajectory import TRACE_SCHEMA
from .routes.session import RouteEnv

class BufferedWriter:
    def __init__(self, path, header, max_bytes):
        self.raw_stream = Path(path).open('xb', buffering=1024 * 1024)
        self.stream = (gzip.GzipFile(fileobj=self.raw_stream, mode='wb', compresslevel=1, mtime=0)
                       if str(path).endswith('.gz') else self.raw_stream)
        self.previous, self.count, self.bytes = None, 0, 0
        self.closed, self.max_bytes = False, max_bytes
        self.append(header)

    def append(self, record):
        if self.closed:
            raise RuntimeError('Trace already closed')
        payload = canonical({'seq': self.count, 'previous': self.previous, 'record': record})
        digest = hashlib.sha256(payload).hexdigest()
        # Hash the canonical content once; the verifier reconstructs that content.
        line = payload[:-1] + b',"sha256":"' + digest.encode() + b'"}\n'
        if self.bytes + len(line) > self.max_bytes:
            raise RuntimeError('Per-game trace byte limit reached')
        self.stream.write(line)
        self.previous, self.count = digest, self.count + 1
        self.bytes += len(line)

    def close(self, status, reason):
        if self.closed:
            return
        try:
            self.append({'kind': 'footer', 'status': status, 'reason': reason})
        finally:
            self.closed = True
            try:
                self.stream.close()
            finally:
                self.raw_stream.close()


class RecordingEnv(RouteEnv):
    def __init__(self, engine_root, *, trace_path=None, log_mode='none',
                 trace_max_bytes=64*1024**2, **kwargs):
        super().__init__(engine_root, trace_path=None, **kwargs)
        if log_mode not in ('none', 'full'):
            raise ValueError('Logging mode must be none or full')
        if log_mode == 'full' and trace_path is None:
            raise ValueError('Full logging requires a trace path')
        self.log_path = Path(trace_path) if trace_path is not None else None
        self.log_mode, self.trace_max_bytes = log_mode, trace_max_bytes
        self.writer = None
        self.closed_status = None
        self.settlement_count = 0

    @property
    def recording(self):
        return self.log_mode == 'full'

    def reset(self, *, seed=1, difficulty='ABYSS'):
        if self.writer is not None or self._state is not None or not self._opened:
            raise RuntimeError('Reset once in an opened environment')
        if self.log_mode == 'full':
            self.writer = BufferedWriter(self.log_path, {
                'kind': 'header', 'schema': TRACE_SCHEMA,
                'identity': {'environment': ENV_SCHEMA, **self.session.identity},
                'config': {'seed': seed, 'difficulty': difficulty,
                           'maxDecisions': self.max_decisions, 'maxAdvances': self.max_advances},
                'source': self.source, 'policyId': self.policy_id,
                'onPolicyTrainingReady': False,
            }, self.trace_max_bytes)
        return super().reset(seed=seed, difficulty=difficulty)

    def _record(self, record):
        # Settlement counts are model inputs, independent of debug recording.
        if ('after' in record and record['before']['phase'] != 'SETTLE'
                and record['after']['phase'] == 'SETTLE'):
            self.settlement_count += 1
        if self.writer is not None:
            self.writer.append(record)

    def close(self, *, error=False):
        try:
            if error or self._failed:
                status, reason = 'error', 'environment-exception'
            elif self._state and self._state['gameEnded']:
                status, reason = 'game-ended', 'natural'
            elif self._reason:
                status, reason = 'truncated', self._reason
            elif self._state and self._state['personalEnded']:
                status, reason = 'personal-ended', 'natural'
            else:
                status, reason = 'interrupted', 'context-closed'
            self.closed_status = status
            if self.writer is not None:
                self.writer.close(status, reason)
        finally:
            super().close(error=error)

    def require_complete(self):
        if (self._opened or self._failed or self._reason or not self._state
                or not self._state['personalEnded']
                or self.closed_status not in ('personal-ended', 'game-ended')):
            raise ValueError('Complete closed natural episode required')
