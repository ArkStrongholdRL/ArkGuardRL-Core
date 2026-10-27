# Current-offer conditional binding

Opt-in planner `binding` schema `arkguardrl-core/current-offer-conditional-binding/v1`
requires active search/adapt upper choices. No new parameters or rewards. A precise
pick records native stream, offer serial/fingerprint, slot and public object.
Missing, stale or illegal identities return the raw policy with a reason receipt.

Sampling draws once from the raw native actor, then maps reward members to the
valid target. Nonreward actions retain their raw timing. Greedy decoding similarly
uses raw argmax before mapping, never argmax on merged masses. `forward` and batch
learning expose the pushforward distribution for PPO, entropy, old KL and anchors.
Covered actions use float32 -1e9 (exp exactly zero); chosen executed logp is the
logsumexp of the legal current reward set, not the original slot score. Upper
choice probability/credit remains separate and is counted once per actual event.

Live rows retain raw selection and scope receipts alongside the candidate map.
The private transport schemas are row/column v8 and worker v11. Scalar/batch and
ordered upper replay reconstruct identity before accepting complete live rows.
No archived or auxiliary-labelled rows are admitted to PPO. Historical source
checkpoints require explicit exact-parameter adoption into the new source identity.
