"""Bounded JSONL transport for the independent native 0.3.1 bridge."""
from __future__ import annotations

import json
import os
from pathlib import Path
from queue import Empty, Queue
import shutil
import subprocess
from threading import Thread

from ..catalog import ENGINE_REVISION
from ..observation import SCHEMA, validate_state


class NativeSession:
    """One native solo match; no policy, reward, action enumeration or automatic decisions.

    Engine/data identity is separate from observations. Old checkpoints are not supported.
    Every protocol/engine failure closes the process and raises an exception.
    """

    def __init__(self, engine_root, *, timeout=30, max_requests=4096,
                 expected_data_sha256=None):
        if timeout <= 0 or max_requests < 1:
            raise ValueError("Positive timeout and request budget required")
        self.engine_root = Path(engine_root).resolve(strict=True)
        self.timeout = timeout
        self.max_requests = max_requests
        self.expected_data_sha256 = expected_data_sha256
        self.identity = self.state = None
        self._process = None
        self._responses = Queue()
        self._count = 0

    def __enter__(self):
        if self._process is not None:
            raise RuntimeError("Session already open")
        node = shutil.which("node")
        if not node:
            raise RuntimeError("Node 22+ required")
        self._count = 0
        self._responses = Queue()
        # An old reader must never write into a later session's queue.
        responses = self._responses
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("SP_", "ARK_", "COOP_", "STRONGHOLD_"))
               and key not in ("NODE_OPTIONS", "NODE_PATH")}
        self._process = subprocess.Popen(
            [node, str(Path(__file__).with_name("native031.mjs")), str(self.engine_root)],
            cwd=self.engine_root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        process = self._process

        def read():
            try:
                for line in process.stdout:
                    responses.put(line)
            finally:
                responses.put(None)

        Thread(target=read, daemon=True).start()
        try:
            self.identity = self._request("identity")["identity"]
            if (self.identity.get("schema") != SCHEMA
                    or self.identity.get("engineRevision") != ENGINE_REVISION
                    or self.identity.get("appVersion") != "0.3.1"):
                raise RuntimeError("Incompatible engine identity")
            if (self.expected_data_sha256 is not None
                    and self.identity.get("dataSha256") != self.expected_data_sha256):
                raise RuntimeError("Engine data identity mismatch")
            return self
        except BaseException:
            self.close()
            raise

    def _request(self, cmd, **payload):
        if self._process is None:
            raise RuntimeError("Use NativeSession as a context manager")
        try:
            if self._count >= self.max_requests:
                raise RuntimeError("Request budget exhausted; game incomplete")
            self._count += 1
            self._process.stdin.write(json.dumps(
                {"cmd": cmd, "requestId": self._count, **payload}, allow_nan=False) + "\n")
            self._process.stdin.flush()
            line = self._responses.get(timeout=self.timeout)
            if line is None:
                raise RuntimeError("Bridge exited before replying; check engine revision/data")
            response = json.loads(line)
            if response.get("requestId") != self._count or response.get("ok") is not True:
                raise RuntimeError(response.get("error", "Invalid bridge response"))
            if "state" in response:
                state = response["state"]
                validate_state(state)
                self.state = state
            return response
        except Empty as exc:
            self.close()
            raise TimeoutError("Engine response deadline exceeded") from exc
        except BaseException:
            self.close()
            raise

    def reset(self, *, seed=1, difficulty="ABYSS"):
        return self._request("reset", seed=seed, difficulty=difficulty)["state"]

    def observe(self):
        return self._request("observe")["state"]

    def act(self, action):
        return self._request("act", action=action)

    def advance(self, *, max_steps=256):
        return self._request("advance", maxSteps=max_steps)

    def close(self):
        process, self._process = self._process, None
        if process is None:
            return
        try:
            try:
                process.stdin.close()
            except BrokenPipeError:
                pass
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        finally:
            process.stdout.close()

    def __exit__(self, *_):
        self.close()
