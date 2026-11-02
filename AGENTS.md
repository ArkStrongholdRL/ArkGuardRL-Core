# Scope

Core owns environment/runtime performance; ArkGuardRL owns strategy research.
Keep code and documentation small. Do not add source manifests, binary fingerprints,
release checklists or source-bound checkpoint gates. Preserve model/input
compatibility, memory bounds and valid rollout semantics. Do not run games or
training merely to clean up files or edit documentation.

`main` is frozen at `6af66bf`. Keep multi-env scheduling experiments on
`codex/multi-env-worker`; do not advance or merge into main without user direction.
