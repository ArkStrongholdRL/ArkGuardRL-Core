"""Separate transport extension; vanilla engine and v7 checkpoint loader are unchanged."""
from pathlib import Path
from queue import Queue
from threading import Thread
import os,shutil,subprocess
from arkguardrl.engine.session import NativeSession,ENGINE_REVISION
from arkguardrl.observation import SCHEMA
from ..environment import DeadlineEnv

class RouteSession(NativeSession):
    def __enter__(self):
        if self._process is not None:raise RuntimeError('Session already open')
        node=shutil.which('node')
        if not node:raise RuntimeError('Node22+ required')
        self._count=0;self._responses=Queue();responses=self._responses
        env={k:v for k,v in os.environ.items() if not k.startswith(('SP_','ARK_','COOP_','STRONGHOLD_')) and k not in ('NODE_OPTIONS','NODE_PATH')}
        self._process=subprocess.Popen([node,str(Path(__file__).with_name('route_bridge.mjs')),str(self.engine_root)],cwd=self.engine_root,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,encoding='utf-8',bufsize=1)
        process=self._process
        def read():
            try:
                for line in process.stdout:responses.put(line)
            finally:responses.put(None)
        Thread(target=read,daemon=True).start()
        try:
            self.identity=self._request('identity')['identity']
            if self.identity.get('schema')!=SCHEMA or self.identity.get('engineRevision')!=ENGINE_REVISION or self.identity.get('appVersion')!='0.3.1':raise ValueError('Engine identity mismatch')
            if self.expected_data_sha256 and self.identity['dataSha256']!=self.expected_data_sha256:raise ValueError('Data identity mismatch')
            return self
        except BaseException:self.close();raise

class RouteEnv(DeadlineEnv):
    def __init__(self,engine_root,**kwargs):
        super().__init__(engine_root,**kwargs)
        old=self.session
        self.session=RouteSession(engine_root,timeout=old.timeout,max_requests=old.max_requests,expected_data_sha256=old.expected_data_sha256)
    def geometry(self):
        self._remaining()
        g=self.session._request('routeGeometry')['geometry']
        if g['revision']!=self._state['revision']:raise ValueError('Stale route geometry')
        from arkguardrl.contracts import fingerprint
        g['observation_id']=fingerprint(self._state)
        return g
    def takeover(self):
        self._remaining()
        return self.session._request('teacherTakeover')
