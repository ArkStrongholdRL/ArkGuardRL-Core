#!/usr/bin/env python3
"""Summarize completed collect-only runs without loading models or running games."""
import argparse
import json
from pathlib import Path
from statistics import median


def sample(path):
    summary = json.loads((path / 'summary.json').read_text())
    rounds = summary.get('iterations', [summary])
    if (path / 'failure.json').exists() or summary['state'] != 'complete' or not rounds:
        raise ValueError(f'Incomplete run: {path}')
    if any(r['state'] != 'complete' or r['phase'] != 'collect-only' for r in rounds):
        raise ValueError(f'Expected completed collect-only runs: {path}')
    seconds = sum(r['timing']['collection_seconds'] for r in rounds)
    decisions = sum(r['decisions'] for r in rounds)
    if seconds <= 0:
        raise ValueError(f'Invalid collection time: {path}')
    return dict(run=path.name, decisions=decisions, collection_seconds=seconds,
                decisions_per_second=decisions / seconds,
                entrypoint_seconds=summary['timing']['entrypoint_seconds'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('runs', nargs='+', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    try:
        samples = [sample(path) for path in args.runs]
        statistics = {}
        for key in ('collection_seconds', 'decisions_per_second', 'entrypoint_seconds'):
            values = [s[key] for s in samples]
            statistics[key] = dict(median=median(values), minimum=min(values), maximum=max(values))
        report = json.dumps(dict(samples=samples, statistics=statistics), indent=2, allow_nan=False)
        with args.out.open('x') as stream:
            stream.write(report + '\n')
    except (OSError, ValueError, KeyError, TypeError) as exc:
        parser.error(str(exc))
    print(args.out)


if __name__ == '__main__':
    main()
