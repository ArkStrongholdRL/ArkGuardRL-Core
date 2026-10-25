"""Checksummed native transition archives. Explicit provenance; no PPO probabilities inferred."""
import json
from pathlib import Path

from .contracts import canonical, fingerprint, action_id, decision_id
from .observation import SCHEMA, validate_state

TRACE_SCHEMA = "arkguardrl/native-trajectory-v1"
SOURCES = {"scripted-diagnostic", "rule-teacher", "human", "bc-policy", "ppo-policy"}


class TrajectoryWriter:
    def __init__(self, path, *, identity, config, source, policy_id):
        if source not in SOURCES or not isinstance(policy_id, str) or not policy_id:
            raise ValueError("Explicit source and policy identity required")
        self.path = Path(path)
        self._stream = self.path.open("xb")
        self._previous = None
        self._count = 0
        self._closed = False
        self.append({"kind": "header", "schema": TRACE_SCHEMA, "identity": identity,
                     "config": config, "source": source, "policyId": policy_id,
                     "onPolicyTrainingReady": False})

    def append(self, record):
        if self._closed:
            raise RuntimeError("Trajectory already closed")
        content = {"seq": self._count, "previous": self._previous, "record": record}
        digest = fingerprint(content)
        self._stream.write(canonical({**content, "sha256": digest}) + b"\n")
        self._stream.flush()
        self._previous = digest
        self._count += 1

    def close(self, *, status, reason):
        if self._closed:
            return
        try:
            self.append({"kind": "footer", "status": status, "reason": reason})
        finally:
            self._closed = True
            self._stream.close()


def verify_trajectory(path):
    """Verify integrity and lifecycle completeness, not deterministic engine replay."""
    previous = None
    header = footer = None
    last_state = None
    count = 0
    observer = None
    feedback_stream_bound = False
    with Path(path).open("rb") as stream:
        for seq, line in enumerate(stream):
            row = json.loads(line)
            content = {k: row[k] for k in ("seq", "previous", "record")}
            if row["seq"] != seq or row["previous"] != previous or row["sha256"] != fingerprint(content):
                raise ValueError(f"Broken trajectory hash/order at row {seq}")
            record = row["record"]
            if seq == 0:
                if record.get("kind") != "header" or record.get("schema") != TRACE_SCHEMA:
                    raise ValueError("Missing trajectory header")
                header = record
                if record.get("identity", {}).get("environment") in ("arkguardrl/native-solo-decisions-feedback-v2", "arkguardrl/native-solo-decisions-feedback-intent-v3"):
                    from .feedback import PublicProgress
                    observer = PublicProgress(execution_schema=("arkguardrl/public-execution-report-v1"
                        if record["identity"]["environment"] == "arkguardrl/native-solo-decisions-feedback-v2"
                        else "arkguardrl/public-execution-report-v2"))
            elif record.get("kind") == "header":
                raise ValueError("Duplicate trajectory header")
            if footer is not None:
                raise ValueError("Records after footer")
            if record.get("kind") == "footer":
                footer = record
            if "state" in record:
                if seq != 1 or record.get("kind") != "reset":
                    raise ValueError("Unexpected reset/state record")
                validate_state(record["state"])
                last_state = record["state"]
                if observer is not None:
                    observer.observe(last_state)
            if "after" in record:
                if last_state != record.get("before"):
                    raise ValueError(f"Disconnected transition at row {seq}")
                validate_state(record["after"])
                if record.get("kind") == "action":
                    if record.get("actionId") != action_id(record["action"]):
                        raise ValueError("Action identity mismatch")
                    if record.get("decisionId") != decision_id(record["before"]):
                        raise ValueError("Decision identity mismatch")
                    if type(record.get("accepted")) is not bool:
                        raise ValueError("Missing action acceptance")
                if observer is not None:
                    if record.get("kind") == "action":
                        receipt = record.get("execution")
                        if not isinstance(receipt, dict):
                            raise ValueError("Feedback environment action omitted its execution receipt")
                        stream_id = receipt.get("stream_id")
                        if not isinstance(stream_id, str) or len(stream_id) != 32:
                            raise ValueError("Execution receipt lacks stream identity")
                        if not feedback_stream_bound:
                            observer.stream_id = stream_id
                            feedback_stream_bound = True
                        actual = observer.observe(record["after"], action=record["action"],
                            reply={"accepted": record["accepted"], "result": record.get("result")})
                        if actual != receipt:
                            raise ValueError("Execution receipt does not match actual public action transition")
                    elif record.get("kind") == "advance" and record.get("executed", 0):
                        observer.observe(record["after"])
                last_state = record["after"]
            previous = row["sha256"]
            count += 1
    if not header or not footer:
        raise ValueError("Incomplete trajectory: header/footer required")
    if last_state is None and footer.get("status") != "error":
        raise ValueError("Incomplete trajectory: state required")
    identity = header.get("identity", {})
    if identity.get("schema") != SCHEMA or identity.get("environment") not in ("arkguardrl/native-solo-decisions-v1", "arkguardrl/native-solo-decisions-feedback-v2", "arkguardrl/native-solo-decisions-feedback-intent-v3"):
        raise ValueError("Unknown environment/observation identity")
    if header.get("onPolicyTrainingReady") is not False:
        raise ValueError("Diagnostic archive must not claim on-policy training readiness")
    if header.get("source") not in SOURCES or not header.get("policyId"):
        raise ValueError("Missing source identity")
    status = footer.get("status")
    if status not in ("personal-ended", "game-ended", "truncated", "interrupted", "error"):
        raise ValueError("Invalid completion status")
    if status == "game-ended" and not last_state["gameEnded"]:
        raise ValueError("Game-end footer without game end")
    if status == "personal-ended" and not last_state["personalEnded"]:
        raise ValueError("Personal-end footer without personal end")
    return {"schema": TRACE_SCHEMA, "environment": identity["environment"],
            "public_feedback_verified": observer is not None, "rows": count, "status": status,
            "source": header["source"], "policyId": header["policyId"], "sha256": previous}
