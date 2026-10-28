# Public persistent strategic options

`planner.options` opts into `arkguardrl-core/public-strategic-options/v1` and
requires the finite PREP64 recovery contract. The action-led 12-dimensional
memory remains unchanged. A separate sampled 12-dimensional option state enters
both primitive candidate heads, scheduler and value via a zero-initialized
projection of the target's frozen catalog embedding and 11 public scalars.
Historical native/plan/recovery parameters are adopted exactly under a new
input contract. There are no synthetic engine actions or action overrides.

Once per eligible PREP round, after mandatory choices and before PREP64 recovery,
the selector samples continue, idle/exit, or a target-specific pair,
level-before-merge, or growth option. Horizons and net-debit caps are two bundles:
1 round/12 funds and 2 rounds/24 funds. These are control-memory limits, not hard
limits on game spending. The context clears on expiry, target disappearance,
realized target merge, or reaching the cap, then waits until next round to choose
again. Continue preserves the original deadline/debit, while explicit exit or
replacement is logged. There is no learned within-PREP termination hazard.

Pair targets are owned ordinary families with at least one copy. Level-before-
merge requires an owned ordinary pair and shop level below maximum. Growth
support uses the explicit pilot anchor list from the planner contract, limited
to owned or currently visible affordable anchors. This first pilot remains
Sargon-scoped for growth; it is not a universal covenant planner. It never forces
buying, selling, leveling, retaining a core, or deploying a six-unit roster.

Net debit is the observed positive funds decrease after accepted buy, reward,
refresh or levelUp. It is a public balance proxy, not an exact transaction ledger;
trait refunds can reduce it. Ready/settlement expiration and next-round income
do not count. Ordinary funds still expire: the plan does not bank currency.

The selector has its own trainable encoder of public global/bond/route/recovery
and previous-option features. Target identity embeddings are frozen. Its intent
scheduler normalizes over present continue/idle/pair/level/growth groups, with
conditional normalization over target-duration alternatives. Initial zero final
heads give a uniform present-intent prior, not a guarantee of strategic value.
A separate RNG avoids consuming primitive action sampling draws.

Live rows retain option context plus the pre-choice public input, full menu,
selected index, old distribution and pre-choice critic value. Trace records
include conditional native, option, and their summed joint log probability.
The native capture remains a normalized conditional native distribution. Full
prefix replay independently rebuilds menus, budgets, targets and all contexts.
Transport row/columnar advance to v5 and worker protocol to v8. All tensors save
on CPU. Cross-worker batched inference for the latent selector is unsupported;
local native worker inference and lossless row transport are supported.

Learning uses TWO factorized clipped PPO objectives. Native PPO conditions on
the sampled persistent option, using the unchanged settlement-clock GAE. A
separate pre-choice option critic avoids a chosen-option-dependent baseline.
Option transitions aggregate actual unchanged native rewards until the next
selector event or natural terminal, gamma=1, and lambda per actual settlement.
There is no option-end bonus, penalty, alternate reward, fabricated termination
or separate strategic terminal. The option decoder and critic own a separate
optimizer; fixed raw inputs avoid native encoder-update drift during its replay.
Primitive and selector buffers come from the same fresh complete live episodes;
no historical receipt or human row is admitted. Truncated episodes remain
ineligible for either update under the current collector contract.

This is not a jointly clipped native-plus-option PPO objective. Native exact KL
is conditional on the recorded option; option KL is reported separately. The
pilot sets both entropy coefficients and anchor beta to zero in matched arms,
and retains the original native reward and gamma/GAE contract. No best-point
selection or model promotion is implied by the new interface.
