"""Load the Rust environment and feature kernels."""
from functools import lru_cache

ABI = 'arkguardrl-core/rust-kernels/v4'


@lru_cache(maxsize=1)
def kernel():
    try:
        from . import _rust_routes
    except ImportError as exc:
        raise RuntimeError(
            'Rust runtime unavailable. Install a platform wheel or rebuild '
            'with a Rust toolchain: python -m pip install -e ".[gym]". '
            'The runtime does not silently fall back to Python.') from exc
    if _rust_routes.ABI != ABI:
        raise RuntimeError('Rust runtime ABI mismatch; rebuild required')
    return _rust_routes
