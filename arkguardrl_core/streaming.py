"""Bounded asynchronous row sender over a dedicated private data pipe."""
from collections import deque
from queue import Queue, Full, Empty
from threading import Event, Thread
import hashlib
import time

from .transport import encode_row

FRAME_BYTES = 1024 * 1024


class RowSender:
    def __init__(self, connection, token, ctx, window=4, queue_rows=2):
        if type(window) is not int or not 1 <= window <= 16:
            raise ValueError('Transfer window must be 1..16 frames')
        if type(queue_rows) is not int or not 1 <= queue_rows <= 8:
            raise ValueError('Transfer queue must be 1..8 rows')
        self.connection, self.token, self.ctx = connection, token, ctx
        self.window, self.queue = window, Queue(queue_rows)
        self.stop = Event()
        self.error = None
        self.submitted = 0
        self.stats = dict(wire_bytes=0, raw_bytes=0, tensor_bytes=0, metadata_bytes=0,
                          encode_seconds=0., ack_wait_seconds=0., queue_wait_seconds=0.,
                          transfer_active_seconds=0., max_inflight_frames=0, frames=0)
        self.thread = Thread(target=self._run, daemon=True)
        self.thread.start()

    def _check(self):
        if self.error is not None:
            raise RuntimeError('Live sender failed') from self.error
        if self.stop.is_set() or time.monotonic() >= self.ctx.deadline:
            raise TimeoutError('Live sender cancelled or deadline exhausted')

    def _put(self, item):
        started = time.monotonic()
        while True:
            self._check()
            try:
                self.queue.put(item, timeout=.1)
                break
            except Full:
                pass
        self.stats['queue_wait_seconds'] += time.monotonic()-started

    def submit(self, row):
        if row.transition is None:
            raise ValueError('Only finalized bootstrap rows can be staged')
        self._put((self.submitted, row))
        self.submitted += 1

    def _ack(self, inflight):
        started = time.monotonic()
        while not self.connection.poll(.1):
            self._check()
        received = self.connection.recv()
        self.stats['ack_wait_seconds'] += time.monotonic()-started
        if received != inflight.popleft():
            raise ValueError('Live frame acknowledgement mismatch')

    def _run(self):
        inflight = deque()
        try:
            while True:
                try:
                    item = self.queue.get(timeout=.1)
                except Empty:
                    if inflight:
                        self._ack(inflight)
                    self._check()
                    continue
                if item is None:
                    while inflight:
                        self._ack(inflight)
                    return
                index, row = item
                started = time.monotonic()
                payload, sizes = encode_row(row, self.ctx.config['max_episode_bytes'])
                self.stats['encode_seconds'] += time.monotonic()-started
                del row, item
                for key in ('raw_bytes', 'tensor_bytes', 'metadata_bytes'):
                    self.stats[key] += sizes[key]
                self.stats['wire_bytes'] += len(payload)
                if max(self.stats['raw_bytes'], self.stats['wire_bytes']) > self.ctx.ppo.max_buffer_bytes:
                    raise MemoryError('Worker live transport exceeds buffer bound')
                digest = hashlib.sha256(payload).hexdigest()
                for offset in range(0, len(payload), FRAME_BYTES):
                    self._check()
                    if len(inflight) >= self.window:
                        self._ack(inflight)
                    self.connection.send(dict(kind='frame', token=self.token, index=index,
                        length=len(payload), offset=offset, sha256=digest,
                        payload=payload[offset:offset+FRAME_BYTES]))
                    inflight.append(('ack', self.token, index, offset))
                    self.stats['frames'] += 1
                    self.stats['max_inflight_frames'] = max(self.stats['max_inflight_frames'], len(inflight))
                self.stats['transfer_active_seconds'] += time.monotonic()-started
                del payload
        except BaseException as exc:
            self.error = exc

    def finish(self):
        self._put(None)
        while self.thread.is_alive():
            self._check()
            self.thread.join(timeout=.1)
        self._check()
        return dict(self.stats)

    def abort(self):
        self.stop.set()
        self.thread.join(timeout=.2)
