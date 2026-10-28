# Finite PREP64 capacity recovery

The opt-in `planner.recovery` contract is
`arkguardrl-core/prep64-single-chess-finite-recovery/v1`. It changes execution
support, not native legality, rewards, gamma, GAE or complete-episode admission.
It applies only after 64 PREP attempts, with one unequipped temporary chess,
no mandatory choice, and native `canReady=false`. Other temporary inventories
raise a technical failure. Earlier PREP choices retain their original support.

A PREP-scoped memory admits a single occupied-board replacement, direct sale of
the temporary chess, sale of an unequipped hand chess (native recompute drains
temp into the resulting hand slot), or sale of a compatible unequipped board
chess followed by deployment into empty capacity. After a replacement/space
choice, only the exact remaining temporary UID's disposal/empty placement or a
hand sale that releases it may follow. Ready is the only candidate once the
obligation is resolved. Three attempted actions including Ready is the bound
for this explicitly admitted short-path class. Rejections consume this same
budget and are excluded across state changes. UID replacement never resets it.
No covenant, rarity, price or strength preference chooses what to retain.

Unexpected new temporary UIDs, equipment returns, mandatory choices or phase
changes invalidate the path and stop sampling. This is a technical cutoff,
never a synthetic personal death, forced Ready or terminal reward. It does not
claim three-step completion for arbitrary inventory states or preserve every
engine-legal option involving equipped units. No training is admitted after a
recovery failure under the unchanged complete-natural-episode contract.

Candidates are restricted before encoding/inference. Native hierarchical heads
therefore recompute conditional action distributions and the scheduler support
from the same restricted candidates for actor, old-policy replay and BC anchor.
No sampled action is replaced. A singleton has probability one. This opt-in
contract uses true joint entropy; historical contracts retain their existing
entropy objective. PPO numerator/denominator, exact KL and anchor KL operate on
the identical saved candidate support.

Eight recovery input fields (`FIELDS` in `capacity_recovery.py`) are shared by
actor and critic through a zero-initialized context projection. All existing
B-revision13 tensors are adopted exactly; the added projection initially changes
neither logits nor value outside support changes. Checkpoints must match the
new input contract and parameter shapes.

Live rows retain the pre-constraint candidate domain and actual successor.
Verification independently folds native attempt history from the full episode,
rebuilds the native domain, and replays the recovery state and candidate subset.
CPU input transport schemas advance to row/columnar v4 and worker v7. Artifact
receipts and scripted prefix diagnostics are not on-policy training datasets.

The zero-training acceptance run found a pre-existing original B float32
multi-state GEMM replay mismatch (one row beyond the unchanged 1e-5 absolute/
relative criterion at batch sizes 256, 64 and 16; single-state execution passed).
CPU `forward_batch` consequently executes each state separately, then stacks
results with original padding. PPO minibatch size, mean loss, gradients and
optimizer step boundaries are unchanged. GPU execution uses the batched network;
live MPS rollout/replay checks retain the original tolerance. CUDA has the same
execution path but still requires validation on NVIDIA hardware. No parameters
were retrained. See [runtime options](RUNTIME.md) for device selection.
