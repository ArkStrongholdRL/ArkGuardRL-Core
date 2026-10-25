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
