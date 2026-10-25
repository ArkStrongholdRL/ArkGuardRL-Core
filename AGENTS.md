# ArkGuardRL-Core

This private repository owns reusable native environment, candidate/history/feature
contracts, optimized complete-episode sampling, route policies and batched learning.
Fixed experiment seeds, parent checkpoint paths/digests, result tables, coefficient
comparisons, human/teacher datasets and research launchers belong in ArkGuardRL.

The external Stronghold Protocol engine stays pinned to
c9d804f470b8d7422ae04eb66faf671cef21ad52 (app0.3.1); do not vendor/rewrite it or
include assets/data. Preserve GPL attribution and separate resource licensing.
Native arkguardrl files were copied byte-for-byte to preserve their source closure.
New package APIs live in arkguardrl_core; their checkpoints have a new strict schema.
Never bypass historical source/contract checks or silently relabel checkpoints.
Do not import experiments, integrations, scripts, route_policy/model/train via
sys.path manipulation. The framework must install and import without ArkGuardRL.

Do not add tests, fixtures, suites or testing infrastructure. Validate through source
review, package/CLI commands and bounded existing work when authorized. Do not
launch repeated games/gradient comparisons or long training without user intent.
Report actual limitations; extraction does not establish new strategy strength.

Use four-space Python indentation, snake_case, explicit tensor/contract/device
checks and CPU checkpoint serialization. Preserve personal termination versus
budget truncation and admit only complete freshly collected games into this PPO.
Parallel sampling and bridge/headless optimizations are research directions, not
implemented features in this initial extraction. No new run is implied by publishing.
