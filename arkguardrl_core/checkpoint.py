"""New strict checkpoint identity for extracted package APIs.

Historical route checkpoints must be strictly loaded in their frozen research
checkout and explicitly adopted; this loader never rewrites old identities.
"""
from arkguardrl import neural
from arkguardrl.contracts import fingerprint
from .identity import source_hashes
from .models import Policy, SCHEMA
from .routes import residual


def identity(contract, width, route_backend):
    return {'schema': SCHEMA, 'input_contract': residual.contract(contract),
        'width': width, 'route_backend': route_backend,
        'architecture': 'native-v7-width; compact semantic facts; route residual; same parameter shapes',
        'native_sources': neural.source_hashes(), 'core_sources': source_hashes()}


def restore_verified_state(model, state):
    expected = model.state_dict()
    if set(state) != set(expected):
        raise ValueError('Core parameter names differ; no implicit migration')
    for name, value in state.items():
        if value.dtype != expected[name].dtype or value.shape != expected[name].shape:
            raise ValueError('Core parameter shape/dtype mismatch: '+name)
        neural._finite(value, 'core parameter '+name)
    model.load_state_dict(state, strict=True)
    return model.eval()


def save(path, model, contract, metadata):
    ident = identity(contract, model.width, model.route_backend)
    neural.save_payload(path, {'schema': SCHEMA, 'identity': ident,
        'identity_sha256': fingerprint(ident), 'definitions': model.base.definitions.bank,
        'metadata': metadata, 'state_dict': {k: v.detach().cpu().clone() for k, v in model.state_dict().items()}})


def load(path, contract):
    payload = neural.load_payload(path)
    if payload.get('schema') != SCHEMA:
        raise ValueError('Extracted core checkpoint required; adopt verified legacy weights explicitly')
    declared = payload['identity']
    ident = identity(contract, declared['width'], declared['route_backend'])
    if declared != ident or payload['identity_sha256'] != fingerprint(ident):
        raise ValueError('Core input/source/model identity mismatch')
    model = Policy(contract, payload['definitions'], declared['width'], declared['route_backend'])
    return restore_verified_state(model, payload['state_dict']), payload['metadata']
