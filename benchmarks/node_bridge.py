"""Bounded JSON client for the original Node performance baseline."""
import json
from pathlib import Path
import selectors
import subprocess

class Oracle:
    def __init__(self, root, data_dir=None):
        self.root = root
        self.data_dir = data_dir

    def __enter__(self):
        bridge = Path(__file__).with_name('node_session.mjs')
        args = ['node', str(bridge), self.root]
        if self.data_dir:
            args.append(str(self.data_dir))
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        return self

    def call(self, **command):
        self.process.stdin.write(json.dumps(command) + '\n')
        self.process.stdin.flush()
        if not self.selector.select(30):
            raise TimeoutError('Original runtime did not reply within 30 seconds')
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError('Original runtime exited: ' + self.process.stderr.read(2000))
        reply = json.loads(line)
        if not reply.get('ok'):
            raise RuntimeError(reply)
        if reply.get('truncated'):
            raise RuntimeError('Original runtime exhausted its callback budget')
        return reply

    def __exit__(self, *_):
        self.selector.close()
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()
        self.process.stderr.close()

def compare_public(actual, expected, path="terminal"):
    if isinstance(actual, dict):
        for key, value in actual.items():
            if key not in expected:
                raise RuntimeError(f"Missing {path}.{key}")
            compare_public(value, expected[key], path+"."+key)
    elif isinstance(actual, list):
        if len(actual) != len(expected):
            raise RuntimeError(f"Length mismatch: {path}")
        for i, (a, b) in enumerate(zip(actual, expected)):
            compare_public(a, b, f"{path}[{i}]")
    elif actual != expected:
        raise RuntimeError(f"Replay mismatch: {path}: {actual!r} != {expected!r}")
