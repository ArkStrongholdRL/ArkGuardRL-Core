# Extraction checks (2026-10-25)

- All new Python sources compiled through the parser; no automated tests/fixtures added.
- Native byte-preserved files match the extraction manifest digests.
- New Core package imports contain no experiment/top-level script dependencies.
- Native and new Core CLI help execute without starting an engine or training.
- Wheel built with the existing setuptools84.2.0; unpacked-wheel imports succeed
  under isolated Python outside the research source tree. Both native and route
  .mjs bridges are present; no experiment/integration/game packages are included.
- Original width1 BC was loaded by its unchanged strict loader against independent
  expected file/data hashes; all681429 parameters were explicitly adopted, saved
  with the new source-bound schema and reloaded with identical parameter values.
  Zero training updates were performed. This does not establish new forward,
  gradient or gameplay parity, throughput or strategy quality.
- No game simulation, gradient comparison, performance repeat or long run executed.
- At initial extraction, parallel collection and bridge observation/headless
  optimizations remained future work; see the subsequent implementation below.

The initial pip-wheel command could not run because the selected interpreter has
no pip module. Building through its installed setuptools.build_meta succeeded;
no dependency installation or environment mutation was needed.

## Route bridge observation optimization (2026-10-25)

- The Core route bridge now checks engine errors and personal/phase/round stop
  boundaries directly after every scheduler callback, instead of constructing a
  complete observation at every intermediate tick. Reply observations, callback
  order, stop boundaries and revision increments remain unchanged. Act/advance
  entry checks read engine health directly; decision replies include geometry for
  that revision, avoiding another request and view construction.
- Source review and Node syntax checking passed. The byte-preserved native bridge
  and external engine are unchanged. No tests or fixtures were added.
- Existing source-bound checkpoint checks continue to see the modified source;
  historical checkpoints are not silently relabeled.

## Persistent parallel collectors (2026-10-25)

- Configurable spawn workers (CLI default four), one Torch thread each, private
  pipes, frozen policy/anchor per collection, ordered complete-episode assembly.
  Live rows use bounded 1MiB frames with acknowledgement, task/row identity and
  SHA256 checks. Failures cancel the collection without a PPO update.
- Each worker retains a Node process. Successful leases dispose the old match
  and scheduler, reset request accounting, then create a fresh game. Episode
  environment/history/receipt/reward state is recreated separately. Worker process
  groups allow cancellation to clean up Node descendants as well.
- Padded execution has a separate budget from the combined capture/preparation
  budget. Concurrent trace archival cannot break the periodic disk-size scan.
- All Core Python sources parsed, CLI help passed, Node syntax and git diff checks
  passed, and byte-preserved native source digests matched the extraction manifest.
- One explicitly authorized full PPO round completed with four workers, each
  collecting two natural episodes on its same Node PID, followed by one MPS PPO
  update. Existing execution-feedback audits and live probability/value checks
  passed; the saved checkpoint strictly reloaded, all model/optimizer tensors
  were on CPU, and all worker/Node processes exited. No separate old/new bridge
  trajectory comparison or sequential timing baseline was run.
- Experiment seeds, timings and result receipts remain in the research repository:
  `ArkGuardRL/docs/research/20261025/SIMULATION-SPEED.md`.

## Batch replay, active candidates and route reuse (2026-10-25)

- PPO validates live metadata separately, prepares public inputs once, and runs
  pre-update probability/value replay plus post-update exact KL in bounded batches
  on the learner device. Capture replay keeps the original per-row tolerances.
  Input preparation checks all original indices, masked facts, presence links and
  execution budgets directly, without running learned encoder/control layers.
- Expert heads and the route target head evaluate only valid candidates. Each
  candidate uses its assigned expert. An absent expert still receives zero
  gradients so Adam momentum is not silently skipped. Candidate order, logical
  minibatches, shuffle and gradient weighting are preserved. Logit normalization
  still uses padded masks; this is not a fully ragged network implementation.
- Route caches are owned by one episode and bounded by entry counts. Every call
  validates observation/geometry binding. Route identity, full board contents,
  catalog identity and public effect/loadout/deployment context invalidate reuse;
  action reuse additionally binds the full source piece and hand destination.
  Only public geometric inputs/results are cached, never learned activations.
- Four selected archived public states were used for forward-only diagnostics:
  cached/uncached route tensors were exactly equal; old/new batch and scalar
  outputs agreed at rtol/atol 1e-5. Diagnostic history was reconstructed for input
  generation, not replayed as training data. No games or optimizer updates ran.
- Python parsing, imports, CLI help and diff whitespace checks passed. Full PPO
  execution, MPS equivalence, cache invalidation across all game events and speed
  gains have not been measured for these latest changes.

Subsequent authorized single-game validation completed a natural episode and one
MPS PPO iteration. Live metadata/probability/value checks and result checkpoint
strict reload passed, with CPU model serialization. The active-candidate heads,
batch replay/KL and route cache all executed successfully. No matched timing
baseline or updated-policy evaluation was run. Experiment timings and settings
are in `ArkGuardRL/docs/research/20261025/optimization-single-receipt.json`.

## Incremental route summaries and decision-local reuse (2026-10-25)

- Route summaries reuse each unchanged ordered board prefix's compensated
  coverage accumulator, then process only moved/swapped suffix units. Integer
  provider counts replace the quadratic unit-pair support tensor with exact
  self-exclusion. This preserves original floating-point accumulation order;
  coverage is not obtained by subtracting rounded totals.
- Candidate terrain eligibility, sorted cells, structured chess projections and
  destination indices are reused within a decision. Native candidate/structured
  sources are now explicitly marked as derived in the extraction manifest.
- Eight archived public-state diagnostics produced exactly identical candidate
  order, observation/candidate features, routing IDs and route tensors before and
  after the change. One authorized complete single-game MPS PPO round then passed
  live replay/audits and strict result reload with CPU model serialization.
  All actual action, acceptance, native result and before/after state records
  exactly matched the preceding same-input run. No updated-policy evaluation ran.
- The single timing comparison improved collection and total wall time, so the
  implementation was retained. This is one observation, not stable throughput
  evidence. Experiment timings are retained in the research repository under
  `docs/research/20261025/incremental-single-receipt.json`.

## Lossless live-row transport (2026-10-25)

- Private pipe protocol v2 encodes tensors as contiguous CPU bytes with original
  dtype/shape, then compresses the entire row at zlib level1. Every metadata field
  and candidate is retained. SHA256/task/order/frame checks remain; inflated row
  and combined transport budgets are enforced independently of compressed size.
  Decompression rejects incomplete/trailing streams. Pickle metadata remains
  restricted to trusted child pipes, not files or external input.
- One same-input natural game plus MPS PPO update passed live replay/audits,
  strict result reload and CPU model serialization. All 191 actual action/native
  result/before/after-state records matched the preceding run exactly. No
  updated-policy evaluation or repeated timing sweep ran.
- Direct codec checks confirmed tensor byte/dtype/shape equality, independent
  writable output storage and rejection of a truncated compressed stream.
  Python parsing and diff checks passed. The single run reduced payload bytes
  and measured transfer time. Total wall time also changed in other phases;
  it cannot all be attributed to transport.
- Per-experiment measurements remain in
  `ArkGuardRL/docs/research/20261025/transport-single-receipt.json`.


## Four Python collection hotspots (2026-10-25)

- Candidate support/healing queries reuse decision-local integer coverage tables
  with exact removal/swap corrections. Unit/action route keys contain only inputs
  actually read by the public default geometry; state/context binding remains.
- Structured numeric/category/routing templates are reused within a decision.
  A bounded content-keyed action-ID LRU retains exact canonical hashes and does
  not trust mutable object identity or remove capture/receiver validation.
- Run source hashing is polled at one-second intervals and forced at collection,
  learning and save boundaries. Budget/config checks remain frequent; strict
  checkpoint source matching is unchanged. Persistent changes are detected at
  the next check; this is not continuous filesystem monitoring.
- All 191 archived decisions matched old candidate order, IDs, structured features,
  route tensors and routing IDs exactly. These archived diagnostics were not
  training input. 1000 randomized support/removal/swap comparisons passed, plus
  action mutation/type/fallback checks and source interval/forced/deadline checks.
- An initial complete game/PPO run passed but was slower than the old historical
  timing even in unchanged phases. An adjacent frozen-baseline/optimized pair
  resolved the concern: both collection and total wall time improved. All three
  actual 191-action/result/state sequences matched the earlier run; each round
  completed two updates. Both optimized checkpoints strictly reloaded, with CPU
  model/optimizer storage. No updated-policy gameplay evaluation was performed.
- This is one sequential pair on a shared machine, not a throughput guarantee.
  Measurements and limitations are recorded in the research repository at
  `docs/research/20261025/hotspots-single-receipt.json` and
  `docs/research/20261025/HOTSPOTS-OPTIMIZATION.md`.


## Decision snapshots and route deltas (2026-10-25)

- State validation returns its freshly generated canonical bytes. Capture cloning,
  observation identity and geometry binding reuse those bytes within the current
  call; there is no persistent mutable-state digest cache or validation bypass.
  Decisions retain immutable history bytes from feature encoding, with the prior
  capture path retained for manually constructed decisions lacking those bytes.
- Route summaries consume removed base indices and ordered appended units. They
  assemble retained indices once per removal group without rebuilding/scanning
  complete per-candidate Python boards. UID/index/capacity guards and compensated
  accumulation order remain intact.
- All 191 archived decision outputs matched exactly. 10400 randomized delta/full
  board summaries matched exactly, including empty boards, re-appended base
  objects and wide DPS magnitudes. Invalid deltas, capture ownership isolation,
  history fallback compatibility and state-mutation rejection checks passed.
- A matched frozen-old/new full-game MPS PPO pair completed with identical 191
  action/result/state sequences and two updates each. Initial weights matched;
  the new result strictly reloaded with CPU model/optimizer storage. Archived
  diagnostics were never used for training; no updated-policy game was played.
- Total and collection wall time improved, but unchanged optimization/compression
  phases also sped up; the full reduction cannot be attributed to this patch.
  An alternating same-process capture microbenchmark also improved, with exact
  non-time output equality. Measurements and limitations are in the research
  repository's `docs/research/20261025/snapshots-deltas-receipt.json`.


## Candidate arrays, feature validation and move keys (2026-10-25)

- Route rows use preallocated float64 arrays, bulk summary subtraction and finite
  checks, then gather aliases and convert to the original float32 tensors. The
  output owns storage independent of persistent cache entries. Candidate order,
  duplicate-row delta semantics and compensated summaries remain unchanged.
- Candidate construction normalizes numeric features once. Only the identical
  immutable tuple verified in the same PID can reuse that numeric check later.
  Dimensions/contracts/continuity remain checked. The private witness is omitted
  from asdict/pickle; received, replaced and foreign-PID values are revalidated.
- Exact standard board/hand move shapes use fixed-field action keys in the same
  bounded LRU. Extended fields and other scalar types use the generic path; the
  canonical digest algorithm and all action content are preserved.
- 191 archived decisions matched old IDs/order, structured/routing features and
  route tensors exactly. Targeted checks covered cache/alias/storage/empty rows,
  nonfinite values, key mutation/types/extensions, feature replacement and
  cross-serialization/PID validation. Archived rows were not used for PPO.
- One frozen baseline and initial optimized round completed; after adding an
  exact string-type fallback guard, the final source completed another full
  game/PPO round. All three 191-action/result/state sequences matched, with two
  updates per round. Final-source checkpoint reload and CPU model/optimizer
  storage passed. No updated-policy gameplay evaluation was performed.
- Collection and whole-process measurements improved, subject to shared-machine
  and MPS noise. Detailed receipts are in the research repository at
  `docs/research/20261025/array-validation-receipt.json`.

## Concurrent collection and persistent rounds (2026-10-25)

- Actor and anchor cache separately by checkpoint bytes. Binding a loaded actor
  checks a strict load receipt against current source, file and parameter hashes
  without constructing another model. Each episode owns a CPU Generator; RNG
  protocol v1 deliberately changes historical seeded trajectories, while worker
  count, model-cache state and batching do not consume that episode's RNG.
- Dedicated control/inference and data pipes support a bounded background sender
  (default four outstanding 1 MiB frames, two queued rows). Only finalized
  bootstrap rows may stream. The parent quarantines rows until natural episode
  termination, public audit and complete ordered transfer have all succeeded.
  Workers retain their episode bank for audit; streaming is not a memory-saving
  claim. Byte budgets are allocation estimates, not aggregate RSS limits.
- `--iterations` partitions unique explicit seeds into equal consecutive groups.
  Worker/Node processes and optimizer state persist; updated actors reload while
  an unchanged anchor stays cached. Intermediate checkpoints and training states
  have unique names, preserving exclusive artifact creation. A reported worker
  failure gets bounded graceful cleanup before forceful shutdown escalation.
- Optional CPU/MPS cross-worker inference preserves candidates, uses bounded
  memory splitting and explicit wait times, and samples in each worker. It stays
  opt-in: measured batches were too small to offset request transport/waiting.
- A fixed eight-game workload (1519 decisions) produced identical action/result/
  state digests at 1/2/4/8 workers, with either transfer mode and CPU/MPS batched
  inference. Both batched backends also completed two-game MPS PPO smoke rounds
  including the existing live probability/value replay checks. No archived
  samples were used for training. Final source completed two PPO rounds/four
  live games: same worker/Node PIDs, second-round actor reloads only, Adam steps
  advancing from 4 to 8, distinct updated weights, strict CPU checkpoint and
  optimizer reload. A separate fixed-policy two-round run reloaded neither
  model nor Node on its second round and matched the same seed trajectories.
- Focused checks covered multi-frame ordering/window bounds, wrong/missing ACKs,
  deadline cleanup, unfinished bootstrap rejection, inference packet byte/shape/
  dtype/field guards and independent storage, parameter/file tampering, strict
  source rejection, and a real truncated episode that discards staged rows and
  closes its workers. Binding preserved global Torch RNG without another load.
- Process-tree RSS was sampled externally every 200 ms, including Node. The
  1/2/4/8-worker collection sweep took 44.24/26.40/19.03/18.19 s; including process
  startup/exit, four workers finished before eight and used less memory. These
  are current scaling results, not speedups over the pre-existing worker pool.
  Transfer timing varied between paired runs; do not generalize the first pair's
  23% reduction. All paired results, terminal drain, inference payloads, memory,
  source identities and final round timings are retained in the research repo's
  `results/runtime-concurrent-20261025` and concurrent-collection report.
  Three stream/episode pairs measured collection at 19.03/24.71, 18.88/19.05,
  and 26.38/27.03 s. Final-source repeat terminal-drain sums were 0.17/5.82 s.
  The alternating binding microbenchmark measured 4.96 ms for receipt checks
  versus 225.36 ms for a current strict reload plus parameter comparison; this
  isolates binding, not historical end-to-end performance. Full records are at
  `docs/research/20261025/CONCURRENT-COLLECTION.md` in the research repository.

## Public feature contexts and numeric tables (2026-10-25)

- Route layouts own bounded numeric unit/coverage tables keyed by all consumed
  values; mutable input changes miss the cache, layout changes invalidate it.
  Board support tables persist only for the exact bound board context. Ordered
  compensated sums remain; final bounded count divisions are vectorized.
- Structured movement candidates resolve references/semantics directly, avoiding
  per-candidate descriptor objects; other actions reuse decision-local descriptors.
  Catalog identity checks validate the same boundary without copying projections;
  the public record API continues returning independent copies.
- All 297 recorded states matched frozen candidate order, structured inputs and
  route tensors exactly; 5400 randomized delta boards matched, including cache
  eviction, mutated numeric inputs and wide DPS magnitudes. Invalid deltas/layout
  reuse were rejected. Archived states were used only for equivalence checks.
- Alternating feature microbenchmark: 4.09 s to 3.50 s (about 14%); no whole-game
  speedup inferred from that ratio. Frozen and final complete live game/MPS PPO
  runs matched 297 actions/states, with four updates each. Detailed receipts are
  in research `results/runtime-feature-blocks-20261025`.

## Columnar live blocks and borrowed PPO preparation (2026-10-25)

- Pipe v4 carries bounded blocks (default 8, maximum 32 live rows). Each tensor
  field has an owned contiguous CPU slab and validated ragged shape/offset table;
  decoded row views do not overlap. Full observations, transitions, candidates,
  geometry and audit metadata remain. The legacy row codec is selectable with
  `--transport-rows 1`. Partial blocks flush at episode completion; unfinished
  episodes still cannot reach PPO. This is not cross-process shared memory.
- PPO preparation borrows retained live tensors for the synchronous update;
  collation copies execution blocks and never mutates the source bank. Generic
  prepare still defaults to independent copies. Live capture/preparation and
  decompression budgets remain enforced; they do not bound total process RSS.
  The persistent board support cache also has an explicit eviction bound.
- Codec checks covered exact legacy values/dtypes/metadata, ragged/empty and
  noncontiguous inputs, storage lifetime after packet release, nonoverlapping
  slices, invalid offsets/shapes/dtypes/duplicate fields and byte limits. Sender
  checks covered full/partial blocks, bounded frames/queue, ordering, wrong and
  absent ACKs, and shutdown. A real truncated game discarded eight rows already
  staged in the parent and closed the worker without returning training data.
- Matched live games/PPO and two eight-game, four-worker pairs preserved behavior;
  selected rollout log probabilities and values also matched the frozen baseline
  exactly. Final checkpoints/optimizers strictly reloaded with CPU storage. Two
  persistent PPO rounds completed 923 decisions and eight updates, reusing worker
  and Node PIDs and preserving Adam steps 4 -> 8.
- On one freshly collected 297-decision batch, copied versus borrowed CPU PPO
  updates were bitwise equal; inputs were unchanged after all updates. MPS weights
  were not bitwise reproducible even for identical-mode repeated updates (maximum
  difference 3.96e-5 in this check); no bitwise GPU weight claim is made. Existing
  pre-update probability/value replay checks passed in every full PPO run.
- Eight-game payload fell from about 93.66 to 53.33 MiB, frames 1519 -> 195.
  End-to-end collection varied: 17.56 -> 17.73 s in one pair, 17.33 -> 16.62 s in
  reverse order. Encoding/decoding costs fell in both pairs. Four-worker peak RSS
  slightly increased (final pair 4.56 -> 4.70 GiB); reduced copying is not a claim
  of lower aggregate RSS. Final single-game entrypoint was 14.24 s; two-round
  entrypoint was 29.98 s. All measurements/limitations are in research
  `docs/research/20261025/FEATURE-BLOCKS.md` and its JSON receipt.
