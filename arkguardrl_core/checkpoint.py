"""New strict checkpoint identity for extracted package APIs.

Historical route checkpoints must be strictly loaded in their frozen research
checkout and explicitly adopted; this loader never rewrites old identities.
"""
from pathlib import Path
from arkguardrl import neural
from arkguardrl.contracts import fingerprint
from .identity import source_hashes
from .io import sha
from .models import Policy, SCHEMA
from .routes import residual


def identity(contract, width, route_backend, specialist=None):
    result = {'schema': SCHEMA, 'input_contract': residual.contract(contract),
        'width': width, 'route_backend': route_backend,
        'architecture': 'native-v7-width; compact semantic facts; route residual; same parameter shapes',
        'native_sources': neural.source_hashes(), 'core_sources': source_hashes()}
    if specialist is not None:
        from .specialist import validate
        validate(specialist)
        result['specialist'] = dict(specialist)
        result['architecture'] = 'frozen native-width route policy; bounded public covenant actor/value residual'
    return result


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
    ident = identity(contract, model.width, model.route_backend, getattr(model, 'specialist_config', None))
    neural.save_payload(path, {'schema': SCHEMA, 'identity': ident,
        'identity_sha256': fingerprint(ident), 'definitions': model.base.definitions.bank,
        'metadata': metadata, 'state_dict': {k: v.detach().cpu().clone() for k, v in model.state_dict().items()}})

    from .sampling import weights_hash
    model._core_load_receipt = (str(Path(path).resolve()), sha(path), fingerprint(ident), weights_hash(model))


def load(path, contract):
    file_sha = sha(path)
    payload = neural.load_payload(path)
    if payload.get('schema') != SCHEMA:
        raise ValueError('Extracted core checkpoint required; adopt verified legacy weights explicitly')
    declared = payload['identity']
    ident = identity(contract, declared['width'], declared['route_backend'], declared.get('specialist'))
    if declared != ident or payload['identity_sha256'] != fingerprint(ident):
        raise ValueError('Core input/source/model identity mismatch')
    if 'specialist' in declared:
        from .specialist import SpecialistPolicy, CONTEXT_SCHEMA
        if declared['specialist']['schema'] == CONTEXT_SCHEMA:
            from .entity_context import EntityContextPolicy
            policy_type = EntityContextPolicy
        else:
            policy_type = SpecialistPolicy
        model = policy_type(contract, payload['definitions'], declared['width'], declared['route_backend'], declared['specialist'])
    else:
        model = Policy(contract, payload['definitions'], declared['width'], declared['route_backend'])
    restore_verified_state(model, payload['state_dict'])
    if sha(path) != file_sha:
        raise ValueError('Checkpoint changed during strict load')
    from .sampling import weights_hash
    model._core_load_receipt = (str(Path(path).resolve()), file_sha,
                               fingerprint(ident), weights_hash(model))
    return model, payload['metadata']


def verify_bound(model, path, contract):
    """Reuse strict load evidence, but recheck source, file and live parameters."""
    from .sampling import weights_hash
    receipt = getattr(model, '_core_load_receipt', None)
    current = (str(Path(path).resolve()), sha(path),
               fingerprint(identity(contract, model.width, model.route_backend,
                                    getattr(model, 'specialist_config', None))), weights_hash(model))
    if receipt is None:
        restored, _ = load(path, contract)
        if restored._core_load_receipt != current:
            raise ValueError('Live actor differs from the declared checkpoint')
        model._core_load_receipt = current
    elif receipt != current:
        raise ValueError('Strict load receipt differs from current source/file/parameters')
