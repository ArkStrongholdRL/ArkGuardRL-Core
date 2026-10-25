"""Buffered full traces or separately identified compact execution receipts.

Compact archives attest online checks; they cannot reconstruct full observations.
No archived record is accepted as an offline PPO training row.
"""
import hashlib
import gzip
from pathlib import Path

from arkguardrl.contracts import canonical, fingerprint
from arkguardrl.environment import ENV_SCHEMA
from arkguardrl.feedback import PublicProgress
from arkguardrl.observation import validate_state
from arkguardrl.trajectory import TRACE_SCHEMA
from .routes.session import RouteEnv

COMPACT_SCHEMA = 'arkguardrl/compact-live-execution-receipts/v1'


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
    def __init__(self, engine_root, *, trace_path, log_mode, trace_max_bytes, **kwargs):
        super().__init__(engine_root, trace_path=None, **kwargs)
        if log_mode not in ('full', 'compact'):
            raise ValueError('Explicit full or compact logging required')
        self.log_path, self.log_mode = Path(trace_path), log_mode
        self.trace_max_bytes = trace_max_bytes
        self.writer = None
        self.observer = PublicProgress()
        self.stream_bound, self.last_state = False, None
        self.rounds, self.moves = {}, 0
        self.receipt_count = 0
        self.closed_status = None

    def reset(self, *, seed=1, difficulty='ABYSS'):
        if self.writer is not None or self._state is not None or not self._opened:
            raise RuntimeError('Reset once in an opened recording environment')
        self.writer = BufferedWriter(self.log_path, {
            'kind': 'header',
            'schema': TRACE_SCHEMA if self.log_mode == 'full' else COMPACT_SCHEMA,
            'identity': {'environment': ENV_SCHEMA, **self.session.identity},
            'config': {'seed': seed, 'difficulty': difficulty,
                       'maxDecisions': self.max_decisions, 'maxAdvances': self.max_advances},
            'source': self.source, 'policyId': self.policy_id,
            'onPolicyTrainingReady': False,
        }, self.trace_max_bytes)
        return super().reset(seed=seed, difficulty=difficulty)

    def _record(self, record):
        if self.writer is None:
            raise RuntimeError('Recording requires an initialized header')
        if self.log_mode == 'full':
            # Full mode retains the independent native post-read verifier.
            self.writer.append(record)
            return
        kind = record['kind']
        if kind == 'reset':
            self.observer.observe(record['state'])
            self.last_state = record['state']
        elif 'after' in record:
            if self.last_state != record['before']:
                raise ValueError('Disconnected live native transition')
            validate_state(record['after'])
            if kind == 'action':
                receipt = record['execution']
                if not self.stream_bound:
                    stream_id = receipt.get('stream_id')
                    if not isinstance(stream_id, str) or len(stream_id) != 32:
                        raise ValueError('Missing execution stream identity')
                    self.observer.stream_id = stream_id
                    self.stream_bound = True
                actual = self.observer._observe_validated(record['after'], action=record['action'],
                    reply={'accepted': record['accepted'], 'result': record['result']})
                if actual != receipt:
                    raise ValueError('Independent online public feedback mismatch')
                self.receipt_count += 1
                self._management(record)
            elif kind == 'advance' and record['executed']:
                self.observer._observe_validated(record['after'])
            self.last_state = record['after']
        if self.log_mode == 'compact':
            record = self._compact(record)
        self.writer.append(record)

    def _management(self, record):
        before, after, action = record['before'], record['after'], record['action']
        self.moves += action['t'] == 'g.move'
        if before['phase'] != 'PREP':
            return
        row = self.rounds.setdefault(before['round'],
            {'used': 0, 'returns': 0, 'seen': set(), 'ready': None})
        row['seen'].add(fingerprint(before['own']))
        if action['t'] == 'g.move':
            row['returns'] += fingerprint(after['own']) in row['seen']
        if action['t'] == 'g.ready' and record['accepted']:
            own = before['own']
            row['ready'] = {
                'prep_used': row['used'], 'budget_hit': row['used'] >= 64,
                'funds': own['funds'], 'deploy_count': own['deployCount'],
                'deploy_cap': own['deployCap'], 'temp_used': sum(p is not None for p in own['temp']),
                'unequipped_items': sum(p is not None and p['kind'] == 'item' for p in own['hand']),
                'affordable_shop_slots': sum(s is not None and not s['sold'] and s['price'] <= own['funds']
                                            for s in own['shop']['slots'])}
        row['used'] += action['t'] != 'g.choice'

    def _compact(self, record):
        kind = record['kind']
        if kind == 'route-input':
            return {k: v for k, v in record.items() if k != 'geometry'}
        if kind == 'reset':
            return {'kind': kind, 'observationId': fingerprint(record['state'])}
        if 'after' not in record:
            return record
        before, after = record['before'], record['after']
        result = {k: v for k, v in record.items() if k not in ('before', 'after')}
        result.update(round=before['round'], phase=before['phase'],
            afterRevision=after['revision'], personalEnded=after['personalEnded'],
            gameEnded=after['gameEnded'])
        # Execution receipts already bind action before/after observation hashes.
        # Retain one useful formation/economy snapshot per accepted Ready.
        if kind == 'action' and record['action']['t'] == 'g.ready' and record['accepted']:
            own = before['own']
            result['ready'] = self.rounds[before['round']]['ready']
            result['formation'] = {k: own[k] for k in ('board', 'bonds') if k in own}
        return result

    def close(self, *, error=False):
        try:
            if self.writer is not None:
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
                self.writer.close(status, reason)
        finally:
            super().close(error=error)

    def audit(self):
        if not self.writer or not self.writer.closed or self.closed_status not in ('personal-ended', 'game-ended'):
            raise ValueError('Complete closed natural episode required')
        if not self.last_state or not self.last_state['personalEnded'] or not self.stream_bound:
            raise ValueError('Missing natural endpoint or verified action stream')
        return {'schema': COMPACT_SCHEMA if self.log_mode == 'compact' else TRACE_SCHEMA,
                'status': self.closed_status, 'validation': 'independent-online-public-feedback',
                'public_feedback_verified': True, 'executions': self.receipt_count,
                'archive_reconstructs_states': self.log_mode == 'full',
                'rows': self.writer.count, 'sha256': self.writer.previous}

    def behavior(self):
        return {'prep_rounds': len(self.rounds), 'moves': self.moves,
            'budget_hits': sum(bool(r['ready'] and r['ready']['budget_hit']) for r in self.rounds.values()),
            'state_returns': sum(r['returns'] for r in self.rounds.values()),
            'ready_rounds': {str(k): v['ready'] for k, v in self.rounds.items()},
            'observed_final_leaks': self.last_state['own']['stats']['leaks']}
