# Explicit public goal-to-candidate relations

`planner.relations` enables `arkguardrl-core/public-goal-candidate-relations/v1`
only with active strategic options and finite PREP64 recovery. It is a new
input/controller configuration.

`goal_relations.encode` produces one 20-bit row per actual candidate, after the
sampled option and recovery constraints. It describes target-family purchase,
non-target purchase, an ordinary triple's public copy-count condition, target
placement/withdrawal/sale/equipment, displacement/replacement, unrelated hand or
board sale, Ready/refresh with target purchase candidates, levelUp during a
level plan, and goal mode/availability. These are public structural features,
not an engine legality oracle or combat forecast. Triple/space flags do not
promise every native side effect or modified promotion rule. No price, rarity,
covenant preference or minimum retention rule chooses an action.

Independent zero-output economy and placement residuals score these bits. Their
masked mean/max also enter a zero-output intent scheduler. Existing hierarchical
normalization remains unchanged: candidate deltas normalize within intents and
the schedule learns whether to choose an intent. No candidate is removed and no
sampled action is replaced. The critic keeps the existing public state and goal
inputs; it does not consume action-specific relation bits, so truncation value
and terminal/bootstrap contracts remain unchanged.

Both matched experiment arms retain the exact raw relation tensor. The disabled
arm replaces it by zeros at the scoring interface; model/optimizer shapes match.
Original option encoder, option actor, option scheduler and pre-choice critic
are frozen in both arms. Their separate update API explicitly returns frozen
without learning under this contract. The native decision layers and relation
heads learn from fresh complete native episodes only. With the selector fixed,
its probability cancels from the joint old/new ratio at a recorded state.
State-dependent menus/goals can still diverge when primitive behavior changes;
freezing parameters does not mean forcing identical goals throughout two games.

Full prefix reconstruction plus direct relation re-encoding binds tensors to the
public state, actual captured action support and selected option. Native trace
inputs include a relation SHA. CPU tensor transport advances to row/columnar v6,
worker v9. Local selector sampling, fixed Goliath, native reward, gamma1,
settlement lambda, natural-end admission and finite-recovery limits are retained.

Research diagnostics ablate this input channel on live states and optionally
intervene on a same-mode alternate goal from the recorded menu. They inspect
conditional action probabilities only, add no gradients or training rewards,
and are not engine/combat counterfactuals or proof that buying a target is better.
