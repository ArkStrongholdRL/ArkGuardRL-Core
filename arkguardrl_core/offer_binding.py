"""GPL-3.0 current-offer conditional pushforward; raw policy retains timing."""
import math
import torch
from arkguardrl.contracts import fingerprint

SCHEMA = 'arkguardrl-core/current-offer-conditional-binding/v1'
ZERO_LOG = -1e9  # Explicit float32 zero-mass sentinel, as for fixed band policy.


def validate(config):
    if not isinstance(config, dict) or set(config) != {'schema', 'enabled'} or config['schema'] != SCHEMA or type(config['enabled']) is not bool:
        raise ValueError('Explicit current-offer binding contract required')


def offer_key(offer):
    return fingerprint({k: offer.get(k) for k in ('tier', 'source', 'label', 'slots')}) if offer else None


def token(memory, state):
    offer = state['own']['shop'].get('rewardOffer')
    slot = memory.requested_offer
    return dict(stream_id=memory.stream_id, offer_serial=memory.offer_serial,
        offer_fingerprint=offer_key(offer), slot=slot,
        object=dict(offer['slots'][slot]))


def encode(config, state, actions, memory, stream_id):
    validate(config)
    mask = torch.zeros(len(actions), dtype=torch.float32)
    bound = getattr(memory, 'requested_binding', None)
    offer = state['own']['shop'].get('rewardOffer')
    reason = None
    if not config['enabled']: reason = 'disabled'
    elif memory.requested_offer is None: reason = 'no-current-target'
    elif bound is None: reason = 'missing-identity'
    elif bound['stream_id'] != stream_id: reason = 'stream-mismatch'
    elif not offer: reason = 'offer-missing'
    elif bound['offer_serial'] != memory.offer_serial or bound['offer_fingerprint'] != offer_key(offer): reason = 'offer-expired'
    elif bound['slot'] != memory.requested_offer or not 0 <= bound['slot'] < len(offer['slots']): reason = 'slot-mismatch'
    elif bound['object'] != offer['slots'][bound['slot']]: reason = 'object-mismatch'
    else:
        obj = offer['slots'][bound['slot']]
        expected = dict(kind=obj['kind'], family=memory.base(obj))
        if expected != memory.requested_object: reason = 'requested-object-mismatch'
    reward = [i for i, action in enumerate(actions) if action['t'] == 'g.reward']
    target = [i for i in reward if actions[i]['idx'] == memory.requested_offer]
    if reason is None and len(target) != 1: reason = 'target-not-uniquely-legal'
    if reason is None:
        mask[reward] = 1
        mask[target[0]] = 2
    return mask, dict(schema=SCHEMA, enabled=config['enabled'], valid=reason is None,
        reason=reason, target=bound, legal_reward_indices=reward,
        target_index=target[0] if reason is None else None)


def validate_tensor(mask, count, batch=None):
    shape = (count,) if batch is None else (batch, count)
    if not isinstance(mask, torch.Tensor) or mask.dtype != torch.float32 or tuple(mask.shape) != shape or not bool(torch.isfinite(mask).all()) or bool(((mask != 0) & (mask != 1) & (mask != 2)).any()):
        raise ValueError('Binding map must be explicit CPU-compatible candidate codes 0/1/2')
    masks = mask if batch is not None else mask[None]
    if bool((((masks == 2).sum(1) != 1) & (masks != 0).any(1)).any()):
        raise ValueError('Each nonempty binding map requires exactly one target')


def pushforward(raw, mask):
    scalar = raw.ndim == 1
    values = raw[None] if scalar else raw
    codes = mask[None] if scalar else mask
    active = (codes == 2).any(1)
    members = codes > 0
    # Finite dummy for empty reductions; no undefined backward at inactive rows.
    grouped = torch.where(active[:, None], values.masked_fill(~members, -torch.inf), torch.zeros_like(values))
    mass = torch.logsumexp(grouped, 1)
    result = torch.where(codes == 2, mass[:, None], torch.where(codes == 1, values.new_full((), ZERO_LOG), values))
    return result[0] if scalar else result


def decode(raw, mask, *, generator=None, greedy=False):
    validate_tensor(mask, len(raw))
    index = int(raw.argmax()) if greedy else int(torch.multinomial(raw.softmax(0), 1, generator=generator))
    actual = int((mask == 2).nonzero()[0]) if mask[index] > 0 else index
    return index, actual


def verify_row(model, row, memory, probabilities):
    config = model.planner_config.get('binding')
    if config is None:
        if row.reward_binding.numel() or row.binding_receipt is not None: raise ValueError('Unexpected binding data')
        return
    state = row.capture['observation']
    stream = row.capture['history']['progress']['stream_id']
    expected, receipt = encode(config, state, row.capture['actions'], memory, stream)
    if not torch.equal(expected, row.reward_binding): raise ValueError('Binding input identity replay mismatch')
    actual = row.binding_receipt
    if actual is None or any(actual[k] != v for k, v in receipt.items()): raise ValueError('Binding identity witness mismatch')
    raw_index = actual['raw_selected_index']
    if not 0 <= raw_index < len(expected): raise ValueError('Raw selected index outside support')
    mapped = int((expected == 2).nonzero()[0]) if expected[raw_index] > 0 else raw_index
    if mapped != row.capture['index']: raise ValueError('Post-sample action binding mismatch')


def verify_behavior(model, row):
    """Only call while the native actor still equals the sampled checkpoint.

    Upper PPO runs after native PPO and verifies its own unchanged selector;
    binding scope/mapping remains verifiable there, native behavior does not.
    """
    if 'binding' not in getattr(model, 'planner_config', {}):
        return
    raw_index = row.binding_receipt['raw_selected_index']
    with torch.no_grad():
        raw, _ = model.forward_raw(row.inputs())
    if not math.isclose(float(raw[raw_index]), row.binding_receipt['raw_selected_logp'], abs_tol=1e-5, rel_tol=1e-5):
        raise ValueError('Raw actor probability replay mismatch')
    effective = pushforward(raw, row.reward_binding).log_softmax(0)
    if not torch.allclose(effective, row.old_logps, atol=1e-5, rtol=1e-5):
        raise ValueError('Effective executed probability mismatch')
