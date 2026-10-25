# Public PREP history input (2026-10-25)

The opt-in entity-policy schema is
`arkguardrl-core/full-entity-prep-history-residual/v1`. It retains the existing
full public entity pool/Transformer encoder and covenant visibility gate; this
change does not broaden that gate or learn a persistent covenant goal.

Set `prep_history={"schema": "arkguardrl-core/public-prep-high-water-input/v1",
"loss_config": <explicit validated ReadyCost configuration>}` alongside the
existing context settings. For the eight-core-covenant configuration this adds
43 float32 inputs. In configured bond order each contributes PREP-start tier/3,
PREP maximum tier/3, tanh(maximum active layers/layer_scale), current tier/3 and
lost tiers/3. Maximum tier and maximum active layers are independent public
high-water values, matching the existing cost formula. The last three inputs
are normalized gross loss, replacement credit and capped projected ready cost.
Tier values above three are outside this version's normalized input domain.

A fresh episode-local tracker observes only current/past public decision states.
A new PREP round resets its start and high-water values. Inputs outside live
PREP are zero. Projection uses the existing reward coefficients; it executes no
fake action. Evaluation may disable cost charging while keeping this explicit
input configuration. If charging is enabled, its coefficients must match.

The collector adds the tensor before inference and captures an owned CPU copy
for PPO. Scalar and batch paths use the same tensor. Parallel inference, row
and columnar transport carry it explicitly (wire v5; row/columnar codecs v2).
Before PPO, the complete ordered episode prefixes must reproduce each captured
tensor exactly; gaps, repeated/interleaved episodes and changed tensors fail.
No archived trajectory is accepted into PPO by this interface.

External decision callers must own a `ReadyCost(loss_config)` tracker per episode
and add `prep_history.tensor_from_tracker(tracker, decision.state)` to `model.row`
before evaluation. Calling `model.row` alone cannot reconstruct public history.
Missing or incompatible history is rejected; it is never silently zero-filled.

Checkpoint identity includes the new architecture/configuration and full input
contract. Old context schemas keep their old input/parameter sets, but exact
source binding still applies: use their sealed source for strict old loads.
No implicit checkpoint migration is provided. Explicit adoption must first
strictly verify the old source/checkpoint, copy every old parameter exactly and
retain the newly zero-initialized `specialist_history.weight` projection. Save
under a new identity with the source receipt; never relabel the original file.
The new projection has 43*hidden parameters (2752 for hidden64).

Read-only validation used sixteen existing complete public archives: 3187
public decision histories, 182 ready projections matching old reward receipts,
four positive fees totalling 0.1662734805344259, and 76 non-PREP zero inputs.
History mutation was rejected per archive; actual history tensors survived the
inference byte codec exactly. Syntax and CLI checks do not establish new live
collector/learner parity, gradients, training quality or survival improvements.
No tests/fixtures, new gameplay, training or checkpoint promotion were added.

An additional forward diagnostic used three recorded public states, each with
its one recorded action and the existing explicit partial-native-history snapshot
audit convention. It is not full-domain or live-history replay. After strict
old-source export, all original parameters were copied exactly; the added 2752
parameters were zero. Scalar outputs initially matched exactly, scalar/batch
outputs agreed within 1e-5, missing history was rejected, and a new separately
identified initial checkpoint saved/reloaded strictly. Zero optimizer updates.
