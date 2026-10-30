"""Route transport with decision-bound geometry and reusable native processes."""
from pathlib import Path
from queue import Queue
from threading import Thread
import os
import shutil
import subprocess
import time

from arkguardrl.engine.session import NativeSession, ENGINE_REVISION, ENGINE_VERSION
from arkguardrl.observation import SCHEMA
from ..environment import DeadlineEnv


class RouteSession(NativeSession):
    def __enter__(self):
        if self._process is not None:
            raise RuntimeError('Session already open')
        node = shutil.which('node')
        if not node:
            raise RuntimeError('Node22+ required')
        self._count = 0
        self._geometry = None
        self._responses = Queue()
        responses = self._responses
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('SP_', 'ARK_', 'COOP_', 'STRONGHOLD_'))
               and k not in ('NODE_OPTIONS', 'NODE_PATH')}
        self._process = subprocess.Popen(
            [node, str(Path(__file__).with_name('route_bridge.mjs')), str(self.engine_root)],
            cwd=self.engine_root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding='utf-8', bufsize=1)
        process = self._process

        def read():
            try:
                for line in process.stdout:
                    responses.put(line)
            finally:
                responses.put(None)

        Thread(target=read, daemon=True).start()
        try:
            self.identity = self._request('identity')['identity']
            if (self.identity.get('schema') != SCHEMA
                    or self.identity.get('engineRevision') != ENGINE_REVISION
                    or self.identity.get('appVersion') != ENGINE_VERSION):
                raise ValueError('Engine identity mismatch')
            if (self.expected_data_sha256 is not None
                    and self.identity['dataSha256'] != self.expected_data_sha256):
                raise ValueError('Data identity mismatch')
            return self
        except BaseException:
            self.close()
            raise

    def _request(self, cmd, **payload):
        reply = super()._request(cmd, **payload)
        if 'state' in reply:
            self._geometry = reply.get('geometry')
            if self._geometry is not None and self._geometry['revision'] != reply['state']['revision']:
                self.close()
                raise ValueError('Reply geometry revision mismatch')
        return reply

    def route_geometry(self, state):
        geometry = self._geometry
        if geometry is None or geometry['revision'] != state['revision']:
            geometry = self._request('routeGeometry')['geometry']
        if geometry['revision'] != state['revision']:
            raise ValueError('Stale route geometry')
        from arkguardrl.contracts import fingerprint
        return geometry | {'observation_id': fingerprint(state)}


class RouteSessionPool:
    """One sequentially leased Node process; every lease owns a fresh match.

    Only a collector worker owns this pool. An error invalidates the process;
    closing a successful lease disposes the match before another episode starts.
    """
    def __init__(self):
        self.session = None
        self.active = False
        self.starts = 0
        self.start_seconds = 0.0

    def lease(self, engine_root, **options):
        return _EpisodeSession(self, engine_root, **options)

    def acquire(self, lease):
        if self.active:
            raise RuntimeError('Native process already leased')
        if self.session is not None:
            if (self.session.engine_root != lease.engine_root
                    or self.session.expected_data_sha256 != lease.expected_data_sha256):
                raise ValueError('Reusable engine/data identity mismatch')
        else:
            started = time.monotonic()
            self.session = RouteSession(lease.engine_root, timeout=lease.timeout,
                max_requests=lease.max_requests, expected_data_sha256=lease.expected_data_sha256)
            try:
                self.session.__enter__()
            except BaseException:
                self.close()
                raise
            self.starts += 1
            self.start_seconds += time.monotonic()-started
        self.session._count = 0
        self.session.max_requests = lease.max_requests
        self.session.timeout = lease.timeout
        self.session.state = None
        self.session._geometry = None
        self.active = True
        return self.session

    def release(self):
        try:
            if self.session is not None and self.active:
                # release does not advance a game; reserve it in the lease budget.
                if self.session._process is None:
                    self.close()
                else:
                    self.session._request('release')
                    self.session.state = None
                    self.session._geometry = None
        except BaseException:
            self.close()
            raise
        finally:
            self.active = False

    def close(self):
        if self.session is not None:
            self.session.close()
            self.session = None
        self.active = False


class _EpisodeSession:
    def __init__(self, pool, engine_root, *, timeout, max_requests, expected_data_sha256):
        self.pool = pool
        self.engine_root = Path(engine_root).resolve(strict=True)
        self._timeout = timeout
        self.max_requests = max_requests + 1
        self.expected_data_sha256 = expected_data_sha256
        self.backend = None

    @property
    def timeout(self):
        return self.backend.timeout if self.backend is not None else self._timeout

    @timeout.setter
    def timeout(self, value):
        self._timeout = value
        if self.backend is not None:
            self.backend.timeout = value

    def __enter__(self):
        self.backend = self.pool.acquire(self)
        return self

    def __getattr__(self, name):
        if self.backend is None:
            raise RuntimeError('Native episode lease is not open')
        return getattr(self.backend, name)

    def close(self):
        if self.backend is not None:
            try:
                self.pool.release()
            finally:
                self.backend = None


class RouteEnv(DeadlineEnv):
    def __init__(self, engine_root, *, session_pool=None, **kwargs):
        super().__init__(engine_root, **kwargs)
        old = self.session
        options = dict(timeout=old.timeout, max_requests=old.max_requests,
                       expected_data_sha256=old.expected_data_sha256)
        self.session = (session_pool.lease(engine_root, **options) if session_pool is not None
                        else RouteSession(engine_root, **options))

    def geometry(self):
        self._remaining()
        return self.session.route_geometry(self._state)

    def takeover(self):
        self._remaining()
        return self.session._request('teacherTakeover')

    def layout_goal(self, uids, *, include_reserves=False):
        """Current deployed members, deterministic public engine layout teacher."""
        self._remaining()
        goal = self.session._request('layoutGoal', uids=list(uids),
                                     includeReserves=include_reserves)['layout']
        if goal['revision'] != self._state['revision']:
            raise ValueError('Layout goal belongs to another decision')
        return goal

    def summon_goal(self, *, fixed_allies=True):
        """Public token-family slots at the current deployed owner positions."""
        self._remaining()
        goal = self.session._request('summonGoal', fixedAllies=fixed_allies)['layout']
        if goal['revision'] != self._state['revision']:
            raise ValueError('Summon goal belongs to another decision')
        return goal
