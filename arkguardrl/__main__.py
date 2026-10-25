"""Inspect an explicitly selected engine and optionally start a native solo session."""
import argparse
import json
from .engine import NativeSession
from .trajectory import verify_trajectory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine-root")
    parser.add_argument("--verify-trajectory", help="Verify a native archive without launching Node")
    parser.add_argument("--expected-data-sha256")
    parser.add_argument("--reset", action="store_true", help="Start a solo match at INFO_CHECK")
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    if args.verify_trajectory:
        if args.engine_root or args.reset:
            parser.error("Trajectory verification cannot be combined with engine/reset")
        print(json.dumps(verify_trajectory(args.verify_trajectory), indent=2))
        return
    if not args.engine_root:
        parser.error("--engine-root is required for engine inspection")
    with NativeSession(args.engine_root, expected_data_sha256=args.expected_data_sha256) as session:
        result = {"identity": session.identity}
        if args.reset:
            state = session.reset(seed=args.seed)
            result["initial"] = {key: state[key] for key in
                                 ("schema", "phase", "round", "gameEnded", "personalEnded")}
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
