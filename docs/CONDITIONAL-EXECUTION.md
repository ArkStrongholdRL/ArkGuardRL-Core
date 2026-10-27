# Independent conditional execution diagnostic

`arkguardrl_core.conditional_execution` fits explicit real-offer conditions with
cross entropy through the existing full production actor. It does not change the
network, candidates, normalization, native rewards, PPO or GAE. Input rows are
independent auxiliary labels, never on-policy rollout rows. Source-game splits,
real-engine eligibility and final execution witnesses belong to the experiment.

Only existing option projection, search economy relation scorer and search
scheduler are trainable. Upper selection, placement, base and value parameters
remain exact. The optimizer and CPU checkpoints are independent and require
explicit source-bound adoption. `responses` reports full and reward-conditional
probabilities, checks scalar/batch equality and preserves the complete domain.
The diagnostic does not establish strategic value or authorize promotion.
