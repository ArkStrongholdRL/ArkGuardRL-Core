# Extraction and compatibility boundary

The source research checkout was `codex/publish-route-ppo-research`, HEAD
`356981e2297393218afbd5531d4af763c941796c`, with explicit source-hashed working-tree
vector/batch additions. Those additions are not mislabeled as committed HEAD files.

Active native `arkguardrl` source bytes are preserved. Original native contracts,
static definitions, histories, rewards, source hashing and serializers stay explicit.
The retained compat modules are historical helpers, not a legacy package dependency.
No `arknights`, `experiments`, experiment integration directory or fixed-seed launcher
is imported or installed by this repository.

The new `arkguardrl_core` namespace replaces ad hoc top-level imports and sys.path
setup with package-relative imports. Native route projection, width models, compact
facts, route arrays, recording, live collection and batched PPO now install together.
Generic run/PPO/sampler settings replace the original experiment CONFIG and sealed
plan dependency. The experiment's specific parent SHA, data paths, seed lists,
paired-arm orchestration, warmup seed splits and result reports were not copied.

Core route checkpoint schema is new. No historical source digest is rewritten.
For old route weights, use the research adapter to strictly load the original model
in its exact original checkout, verify the externally specified file SHA, construct
an independent expected contract, validate all new parameter names/shapes/dtypes,
then save an explicitly adopted zero-update core checkpoint with migration metadata.
Successful parameter adoption is not a new forward/backward or gameplay parity claim.
Native old checkpoints retain their own original native loader contract.

No engine-level optimization was silently bundled. The external pinned engine is
not copied or changed; the route bridge/teacher geometry code is byte-preserved.
In particular instantCombat=false remains, and intermediate observe/snapshot work
is unchanged. Multi-process sampling remains unimplemented. Implement future changes
here with their own source/config identities; do not update frozen experiment closures.

Basic native BC and PPO are provided by the preserved native learning CLI. Optimized
route learning has reusable Policy, Run, sampling.collect, prepared batch data and
ppo_update APIs plus a bounded one-iteration CLI. This extraction does not add a
route teacher-dataset trainer or promote a stronger teacher; dataset construction
and experiment-specific offline route training remain in the research repository.

Validation policy is no tests/fixtures/test infrastructure. Initial extraction
checks package imports, source closure equality, wheel contents, CLI help and explicit
adoption/serialization using an existing verified parent. No new simulated games,
gradient comparisons, speed repeats, strategy evaluations or long runs are implied.
