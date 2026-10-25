"""Exact source closure for the new framework; no legacy checkpoint relabeling."""
import hashlib
from pathlib import Path


def source_hashes():
    root = Path(__file__).parent
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob('*')) if p.suffix in ('.py', '.mjs') and '__pycache__' not in p.parts}
