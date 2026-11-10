# Scope

Core owns environment/runtime performance; ArkGuardRL owns strategy research.
Maintain only Desperation mode (`mode_single_abyss`). Scope migration work,
new content and acceptance to this mode; other modes are not maintenance targets.
Do not add standard-mode-only content such as enemy-debuff tactical cards.
Keep code and documentation small. Do not add source manifests, binary fingerprints,
release checklists or source-bound checkpoint gates. Preserve model/input
compatibility, memory bounds and valid rollout semantics. Do not run games or
training merely to clean up files or edit documentation.

Multi-environment execution and vectorized runtime work are part of Core and
may be included in PRs targeting main. Continue vectorization work in Core.
