#!/usr/bin/env python3
"""Temporary byte-equivalence oracle for crates/engine/src/identity.rs.

Builds a throwaway crate outside the repository (identity.rs is included with
#[path], so lib.rs does not need to register it yet), replays values collected
from real ArkGuardNativeEnv episodes plus handwritten and fuzzed boundaries,
and compares every result against arkguardrl.contracts.

Run with an environment that has the project's [gym] extras installed, e.g.:

    from benchmarks.oracle_identity import run
    run(data_root="../ArkGuardRL/game/data", library="target/release/lib_native.dylib")

Set include_real=False to check only handwritten/fuzzed samples.
The script writes nothing inside the repository.
"""
import json
import os
from pathlib import Path
import platform
import random
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

RUNNER_SOURCE = r'''#[path = __IDENTITY_PATH__]
mod identity;

use std::io::{self, BufRead, Write};

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn main() {
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: serde_json::Value = serde_json::from_str(&line).expect("request JSON");
        let op = request[0].as_str().expect("op");
        let value = &request[1];
        let body = match op {
            "canonical" => identity::canonical(value)
                .map(|bytes| format!("{{\"hex\":\"{}\"}}", hex(&bytes))),
            "fingerprint" => identity::fingerprint(value)
                .map(|text| format!("{{\"fp\":\"{}\"}}", text)),
            "action" => identity::action_id(value)
                .map(|text| format!("{{\"fp\":\"{}\"}}", text)),
            "decision" => identity::decision_id(value)
                .map(|text| format!("{{\"fp\":\"{}\"}}", text)),
            other => Err(format!("unknown op: {}", other)),
        };
        match body {
            Ok(text) => writeln!(out, "{}", text).expect("stdout"),
            Err(error) => writeln!(
                out,
                "{{\"error\":{}}}",
                serde_json::to_string(&error).expect("error JSON")
            )
            .expect("stdout"),
        }
    }
    out.flush().expect("flush");
}'''


def build_runner(root, keep=False):
    """Compile identity.rs with serde_json into a standalone JSONL filter."""
    build = Path(tempfile.mkdtemp(prefix="arkguard-identity-"))
    source = RUNNER_SOURCE.replace("__IDENTITY_PATH__", json.dumps(str(root / "crates/engine/src/identity.rs")))
    (build / "src").mkdir()
    (build / "Cargo.toml").write_text(
        "[package]\nname = \"identity-oracle\"\nversion = \"0.0.0\"\nedition = \"2021\"\n"
        "[dependencies]\n"
        "serde_json = { version = \"=1.0.149\", features = [\"preserve_order\", \"float_roundtrip\"] }\n",
        encoding="utf-8",
    )
    (build / "src/main.rs").write_text(source, encoding="utf-8")
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(build / "target")
    command = ["cargo", "build", "--release", "--quiet"]
    for extra in (["--offline"], []):
        result = subprocess.run(command + extra, cwd=build, env=environment,
                                capture_output=True, text=True)
        if result.returncode == 0:
            break
    else:
        raise RuntimeError(f"cargo build failed:\n{result.stdout}\n{result.stderr}")
    binary = build / "target/release/identity-oracle"
    if not binary.exists():
        raise RuntimeError(f"missing built runner at {binary}")
    return binary, (None if keep else build)


def run_rust(binary, requests):
    """One subprocess for all requests; returns one parsed response per request."""
    payload = "\n".join(
        json.dumps([request["op"], request["value"]], ensure_ascii=False,
                   separators=(",", ":"), allow_nan=False)
        for request in requests
    ) + "\n"
    result = subprocess.run([str(binary)], input=payload, capture_output=True,
                            text=True, timeout=600)
    if result.returncode != 0:
        raise RuntimeError(f"runner failed ({result.returncode}):\n{result.stderr[-4000:]}")
    lines = result.stdout.splitlines()
    if len(lines) != len(requests):
        raise RuntimeError(f"runner returned {len(lines)} lines for {len(requests)} requests")
    return [json.loads(line) for line in lines]


def python_result(request):
    from arkguardrl import contracts
    try:
        if request["op"] == "canonical":
            return {"hex": contracts.canonical(request["value"]).hex()}
        if request["op"] == "fingerprint":
            return {"fp": contracts.fingerprint(request["value"])}
        if request["op"] == "action":
            return {"fp": contracts.action_id(request["value"])}
        if request["op"] == "decision":
            return {"fp": contracts.decision_id(request["value"])}
        raise ValueError(request["op"])
    except Exception as exc:  # ValueError/KeyError/TypeError are expected refusals
        return {"error": True, "detail": f"{type(exc).__name__}: {exc}"}


def add(requests, seen, label, op, value, group, known=False):
    key = (op, json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    if key in seen:
        return False
    seen.add(key)
    requests.append({"label": label, "op": op, "value": value, "group": group,
                     "known": known})
    return True


def int_bounds(value, low=None, high=None, floats=None):
    """Track the integer range and float presence of collected JSON."""
    if isinstance(value, bool):
        return low, high, floats
    if isinstance(value, int):
        low = value if low is None else min(low, value)
        high = value if high is None else max(high, value)
    elif isinstance(value, float):
        floats = floats + 1
    elif isinstance(value, list):
        for item in value:
            low, high, floats = int_bounds(item, low, high, floats)
    elif isinstance(value, dict):
        for item in value.values():
            low, high, floats = int_bounds(item, low, high, floats)
    return low, high, floats


def choose(environment, step, rng):
    """Deterministic pilot: rounds, legal placements, buys, then filler moves."""
    actions = environment._decision.actions
    own = environment._result.observation["own"]
    if step < 150:
        if step % 4 == 0:
            for index, action in enumerate(actions):
                if action["t"] == "g.choice":
                    return index
        hand = {piece["uid"] for piece in own["hand"] if piece and piece["kind"] == "chess"}
        occupied = {(piece["row"], piece["col"]) for piece in own["board"]}
        moves = [(index, action) for index, action in enumerate(actions)
                 if action["t"] == "g.move" and action["uid"] in hand
                 and action["to"]["area"] == "board" and action.get("dir") == "RIGHT"
                 and (action["to"]["row"], action["to"]["col"]) not in occupied]
        if moves and own["deployCount"] < own["deployCap"]:
            return rng.choice(moves)[0]
        if step % 3 == 0:
            for index, action in enumerate(actions):
                if action["t"] == "g.buy":
                    slot = own["shop"]["slots"][action["slot"]]
                    if slot and slot["kind"] == "chess" and not slot["sold"] \
                            and slot["price"] <= own["funds"]:
                        return index
        pool = [index for index, action in enumerate(actions) if action["t"] != "g.ready"]
        if pool:
            return rng.choice(pool)
    return next(index for index, action in enumerate(actions) if action["t"] == "g.ready")


def collect_real(requests, seen, data_root, library, seeds, max_steps):
    from arkguardrl_core.native_gym import ArkGuardNativeEnv

    states = actions = unique_actions = 0
    low = high = None
    floats = 0
    per_seed = []
    for seed in seeds:
        rng = random.Random(seed)
        environment = ArkGuardNativeEnv(data_root, core="band_chen",
                                        native_library=library)
        episodes_states = episodes_actions = 0
        try:
            environment.reset(seed=seed)
            for step in range(max_steps):
                state = environment._result.observation
                if state["decisionRequired"] and not state["personalEnded"]:
                    episodes_states += 1
                    low, high, floats = int_bounds(state, low, high, floats)
                    add(requests, seen, f"state seed={seed} step={step}", "canonical",
                        state, "real state")
                    add(requests, seen, f"state seed={seed} step={step}", "fingerprint",
                        state, "real state")
                    add(requests, seen, f"state seed={seed} step={step}", "decision",
                        state, "real state")
                    for index, action in enumerate(environment._decision.actions):
                        episodes_actions += 1
                        low, high, floats = int_bounds(action, low, high, floats)
                        label = f"action seed={seed} step={step} #{index}"
                        if add(requests, seen, label, "action", action, "real action"):
                            unique_actions += 1
                            add(requests, seen, label, "canonical", action, "real action bytes")
                            add(requests, seen, label, "fingerprint", action, "real action fingerprint")
                        else:
                            # Check every candidate occurrence, even repeated intents.
                            for op, group in (("action", "real action"),
                                              ("canonical", "real action bytes"),
                                              ("fingerprint", "real action fingerprint")):
                                requests.append({"label": label, "op": op, "value": action,
                                                 "group": group, "known": False})
                if not environment._decision or not environment._decision.actions:
                    break
                _, _, terminated, truncated, _ = environment.step(choose(environment, step, rng))
                if terminated or truncated:
                    break
        finally:
            environment.close()
        states += episodes_states
        actions += episodes_actions
        per_seed.append((seed, episodes_states, episodes_actions))
    return states, actions, unique_actions, per_seed, (low, high, floats)


def manual_samples(requests, seen):
    values = [
        ({"t": "g.move", "uid": 7, "to": {"area": "board", "row": 9, "col": 2},
          "dir": "RIGHT"}, "fast board move"),
        ({"t": "g.move", "uid": 7, "to": {"area": "hand", "idx": 3}}, "fast hand move"),
        ({"t": "g.move", "uid": 7, "to": {"area": "board", "row": 9, "col": 2},
          "dir": "RIGHT", "x": 1}, "board move with extra field"),
        ({"t": "g.move", "uid": True, "to": {"area": "board", "row": 9, "col": 2},
          "dir": "RIGHT"}, "board move bool uid"),
        ({"t": "g.move", "uid": 7, "to": {"area": "board", "row": 9.0, "col": 2},
          "dir": "RIGHT"}, "board move float row"),
        ({"t": "g.move", "uid": 7, "to": {"area": "board", "row": 9, "col": 2},
          "dir": "R" * 257}, "board move long dir"),
        ({"t": "g.move", "uid": 7, "to": {"area": "hand", "idx": 3, "x": 0}},
          "hand move extra field"),
        ({"t": "g.move", "uid": 7, "to": {"area": "temp", "idx": 3}}, "unknown move area"),
        ({"t": "g.buy", "slot": 1}, "buy"),
        ({"t": "g.buy", "slot": 1.0}, "buy float slot"),
        ({"t": "g.buy", "slot": [1]}, "buy list slot"),
        ({"t": "g.buy", "slot": 1, "名": "值"}, "buy unicode field"),
        ({"t": "x.unknown", "a": 1}, "unknown t"),
        ({"t": "g.buy", "slot": 1, **{f"k{i}": i for i in range(15)}}, "17-key action"),
        ({"t": "g.buy", "slot": 1, **{f"k{i}": i for i in range(14)}}, "16-key action"),
        ({"t": "g.buy", "slot": 1, "k" * 300: 1}, "long action key"),
        ({"t": "g.buy", "slot": 1, "deep": {"a": {"b": {"c": 1}}}}, "depth-3 action"),
        ({"t": "g.buy", "slot": 1, "deep": {"a": {"b": 1}}}, "depth-2 action"),
        ({"t": "g.buy", "slot": 1, "neg": -0.0, "big": 1e16}, "action floats"),
        ({"t": "g.buy", "slot": 1, "none": None, "bool": True, "int": 1},
         "action scalar tags"),
        ({"t": "g.move", "uid": 1, "to": {"area": "hand", "idx": 2}, "dir": "LEFT"},
         "hand move with dir"),
        ("g.ready", "action not an object"),
        ({"slot": 1}, "action without t"),
        ({"t": 1, "slot": 1}, "action non-string t"),
        ({"t": "g.ready", "ready": True, "extra": [1, None, True]}, "ready with list"),
    ]
    for value, label in values:
        add(requests, seen, label, "action", value, "manual action")

    states = [
        ({"schema": "arkguardrl/native-solo-decisions-feedback-intent-v3",
          "revision": 3,
          "public": {"round": 2, "phase": "PREP", "serverNow": "2026-01-01T00:00:00Z",
                     "deadline": 123, "overtimeAt": None, "中文键": "值", "nested": {"b": 1, "a": 1.0}},
          "own": {"funds": 10, "hand": [None, {"uid": 1, "kind": "chess"}]},
          "personalEnded": False, "gameEnded": False}, "representative state"),
        ({"schema": "s", "public": {"serverNow": 1, "deadline": 2, "overtimeAt": 3},
          "own": {}, "personalEnded": True, "gameEnded": False}, "only ignored public keys"),
        ({"schema": "s", "public": {"z": -0.0, "a": 1e-7, "m": [None, True, 1, 1.0]},
          "own": {"x": 18446744073709551615}, "personalEnded": False, "gameEnded": True},
         "state scalars"),
        ({"schema": "s", "public": {}, "own": None, "personalEnded": False,
          "gameEnded": False, "extra": "ignored"}, "state extra ignored"),
        ({"schema": "s", "public": {"😀": 1, "É": 2, "中": 3, "a": 4}, "own": [],
          "personalEnded": False, "gameEnded": False}, "state unicode keys"),
    ]
    for value, label in states:
        add(requests, seen, label, "decision", value, "manual decision")
        add(requests, seen, label, "canonical", value, "manual decision")
    missing = [
        ({"schema": "s", "own": {}, "personalEnded": False, "gameEnded": False},
         "state missing public"),
        ({"schema": "s", "public": {}, "personalEnded": False, "gameEnded": False},
         "state missing own"),
        ({"schema": "s", "public": {}, "own": {},
          "personalEnded": False}, "state missing gameEnded"),
    ]
    for value, label in missing:
        add(requests, seen, label, "decision", value, "manual decision error")

    canonical = [
        ({"z": None, "b": True, "i": 0, "neg": -1, "big": 18446744073709551615,
          "s": "text", "f": 1.0, "negzero": -0.0, "tiny": 1e-7, "huge": 1e16,
          "fixed": 1e15, "mid": 0.30000000000000004}, "all scalar kinds"),
        ({"中文": "值", "é": "e", "😀": [1, 2], "Z": "upper", "a": "lower",
          "\u2028": "separator", "\u007f": "del", "\ue000": "private use"},
         "unicode keys and values"),
        ({"k" * 300: 1, "短": 2}, "300-char key"),
        ({"esc": "\"\\\b\f\n\r\t\u0000\u0001\u001f\u007f\u2028\u2029"},
         "string escapes"),
        ({"deep": [{"b": [{"c": None}]}, [[[]]]], "empty": {}, "list": []},
         "nesting"),
        ({}, "empty object"),
        ([], "empty array"),
        ([True, 1, False, 0, None, "1"], "bool and int"),
        ({"u000b": "\u000b", "u001f": "\u001f"}, "low controls"),
        ({"astral": "𝄞😀", "bmp": "中文é"}, "astral text"),
        ({"long": "x" * 1000}, "long string"),
        ({"big neg": -9223372036854775808, "big pos": 18446744073709551615},
         "64-bit integer bounds"),
    ]
    for value, label in canonical:
        add(requests, seen, label, "canonical", value, "manual canonical")
        add(requests, seen, label, "fingerprint", value, "manual canonical")
    add(requests, seen, "beyond u64 integer", "canonical",
        {"n": 123456789012345678901234567890}, "known limitation", known=True)


def float_samples(requests, seen, count, rng):
    values = [1.0, -0.0, 0.0, 0.1, 1e-7, 1e-5, 1e-4, 1e15, 1e16, 1e21, 1e22,
              5e-324, 2.2250738585072014e-308, 1.7976931348623157e308,
              9007199254740993.0, 1234567890123456.0, 1.0000000000000002,
              0.30000000000000004, 123456789.0, 2.0 ** 53, 1.5, -1.5,
              134862484565824.12]  # exact halfway decimal: Python chooses even
    for value in values:
        add(requests, seen, f"float {value!r}", "canonical", value, "float boundary")
    while count > 0:
        bits = rng.getrandbits(64)
        value = struct.unpack("<d", bits.to_bytes(8, "little"))[0]
        if value != value or value in (float("inf"), float("-inf")):
            continue
        count -= 1
        add(requests, seen, f"float bits {bits:016x}", "canonical", value, "float fuzz")


def random_json(rng, depth=0):
    choices = ["null", "bool", "int", "float", "str", "list", "dict"]
    if depth >= 3:
        choices = choices[:5]
    kind = rng.choice(choices)
    if kind == "null":
        return None
    if kind == "bool":
        return rng.random() < 0.5
    if kind == "int":
        return rng.choice([0, 1, -1, 2 ** 31, 2 ** 63 - 1, 18446744073709551615,
                           rng.randrange(-1000, 1000)])
    if kind == "float":
        return rng.choice([0.0, -0.0, 1.0, 1e-7, 1e16, rng.uniform(-1e6, 1e6)])
    if kind == "str":
        alphabet = ["a", "Z", "é", "中", "😀", "\u0000", "\n", "\"", "\\", "\u2028",
                    "\u007f", "𝄞"]
        return "".join(rng.choice(alphabet) for _ in range(rng.randrange(0, 8)))
    if kind == "list":
        return [random_json(rng, depth + 1) for _ in range(rng.randrange(0, 4))]
    keys = ["a", "b", "Z", "é", "中", "😀", "k" * rng.randrange(1, 40), "\u0000"]
    rng.shuffle(keys)
    return {key: random_json(rng, depth + 1)
            for key in keys[:rng.randrange(0, 4)]}


def json_samples(requests, seen, count, rng):
    for _ in range(count):
        value = random_json(rng)
        add(requests, seen, f"json fuzz {json.dumps(value)[:80]}", "canonical",
            value, "json fuzz")
        add(requests, seen, f"json fuzz {json.dumps(value)[:80]}", "fingerprint",
            value, "json fuzz")


def run(*, data_root=None, library=None, seeds=(7, 11), max_steps=400,
        float_count=5000, json_count=300, include_real=True, keep_build=False,
        runner_bin=None, max_differences=20):

    requests, seen = [], set()
    try:
        from arkguardrl import contracts  # noqa: F401  (fail early, clear message)
    except ImportError as exc:
        raise ValueError(f"run from the repository with its dependencies installed: {exc}")

    real = None
    if include_real:
        if data_root is None or not Path(data_root).is_dir():
            raise ValueError(f"missing data root {data_root}; provide data_root or set include_real=False")
        if library is None or not Path(library).exists():
            raise ValueError(f"missing native library {library}; provide library or set include_real=False")
        real = collect_real(requests, seen, data_root, library, seeds,
                            max_steps)

    manual_samples(requests, seen)
    rng = random.Random(20261109)
    float_samples(requests, seen, float_count, rng)
    json_samples(requests, seen, json_count, random.Random(7))

    if runner_bin:
        binary, cleanup = Path(runner_bin), None
    else:
        binary, cleanup = build_runner(ROOT, keep=keep_build)

    try:
        responses = run_rust(binary, requests)
    finally:
        if cleanup is not None:
            shutil.rmtree(cleanup, ignore_errors=True)

    expected = [python_result(request) for request in requests]
    def normalize(result):
        return {"error": True} if "error" in result else result
    groups = {}
    differences = []
    known_differences = []
    for request, want, got in zip(requests, expected, responses):
        bucket = groups.setdefault(request["group"], [0, 0])
        bucket[1] += 1
        if normalize(want) == normalize(got):
            bucket[0] += 1
            continue
        entry = (request, want, got)
        if request["known"]:
            known_differences.append(entry)
        else:
            differences.append(entry)

    print("== coverage ==")
    if real:
        states, actions, unique_actions, per_seed, bounds = real
        print(f"real episodes: {len(per_seed)} seeds " +
              ", ".join(f"seed {seed}: {s} states/{a} candidates"
                        for seed, s, a in per_seed))
        print(f"real states: {states} unique; candidate actions: {actions} occurrences, "
              f"{unique_actions} unique; decision ids: {states}")
        low, high, floats = bounds
        print(f"real integer range: {low}..{high}; real float occurrences: {floats}")
        if low is not None and (low < -(2 ** 63) or high > 2 ** 64 - 1):
            print("WARNING: real data exceeds serde_json's 64-bit integer domain")
    else:
        print("real episodes: skipped")
    print(f"requests: {len(requests)}")

    print("\n== results ==")
    for group, (passed, total) in sorted(groups.items()):
        print(f"{group:20s} {passed:6d}/{total:<6d} matched")
    print(f"TOTAL                {sum(v[0] for v in groups.values()):6d}/"
          f"{len(requests):<6d} matched")

    def show(entries, title):
        if not entries:
            return
        print(f"\n== {title} ({len(entries)}) ==")
        by_group = {}
        for entry in entries:
            by_group.setdefault(entry[0]["group"], []).append(entry)
        for group, items in sorted(by_group.items()):
            print(f"-- {group} --")
            for request, want, got in items[:max_differences]:
                print(f"label: {request['label']}")
                print(f"input: {json.dumps(request['value'], ensure_ascii=False)}")
                print(f"python: {json.dumps(want, ensure_ascii=False)}")
                print(f"rust:   {json.dumps(got, ensure_ascii=False)}")
            if len(items) > max_differences:
                print(f"... {len(items) - max_differences} more in {group}")

    show(differences, "differences")
    show(known_differences, "known limitations")

    if differences:
        print(f"\nFAIL: {len(differences)} unexpected difference(s)")
        return 1
    print("\nPASS: every non-known sample matched byte-for-byte"
          + (f" ({len(known_differences)} known limitation sample(s))"
             if known_differences else ""))
    return 0
