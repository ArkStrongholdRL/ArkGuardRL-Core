# BC-initialized action-led active growth pilot

New opt-in checkpoint planner identity: `arkguardrl-core/action-led-separated-growth/v1`.
Original Policy and old specialist checkpoint identities remain separate. Explicit
verified BC adoption copies all original parameters; fresh plan heads initially
have zero output. Public identity/spatial encoders and original route layers stay
frozen. Shared decision combine, independent base purchase/placement heads,
scheduler and critic open from BC. Additional plan candidate heads are separate
for economy and placement, normalized within native intents; a separate scheduler
sets intent mass. No Sargon visibility gate or bounded logit residual.

Public action-led plan memory is deterministic context, not a sampled latent
option. Accepted buys/rewards that produce a pair establish a pair target;
accepted leveling while pursuing it records the level-then-merge stage; acquiring
a configured public growth anchor establishes a growth target. Targets disappear,
resolve or expire after three rounds. The actor can abandon through its native
inventory decisions. Counts/steps/spending are inputs, not bonuses or constraints.
There is no scripted action override apart from declared legal fixed-band choice.
Plan initiation/selection is implicit in learned native actions, not a separate
learned option policy. Full learned option selection, budgets and termination
are not claimed by this first pilot. Raw IDs come from the public catalog only.

The 12-float plan input is captured on CPU. Every live episode prefix reconstructs
it exactly before PPO. Scalar/batch/inference and row/columnar wire transport carry
the same tensor (private wire v6, row/columnar v3); missing inputs reject. Public
SETTLE entrances are counted even when recording is disabled. New
`PPOConfig.gae_clock='settlement'` retains gamma1 native reward/termination and
uses lambda**actual_settlement_count in backward GAE; within-PREP trace factor1.
Native decision-clock GAE remains available. Bootstrap on a genuine truncation
uses the successor plan-conditioned value; truncated episodes still do not train.

No shaping/reward changes are required. The research pilot declares anchors,
fixed band, training scope, annealing schedule, seeds and budgets. Native actions
are sampled from actual recorded probabilities; no human, teacher, forced plan
or historical archive rows are admitted into live PPO. New schema is a combined
intervention, not a single-factor ablation. No tests/fixtures added.
