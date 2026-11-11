"""Catalog-owned Rust encoder for the current dynamic observation layout."""
from .native_runtime import kernel


def encoder(catalog):
    cached = getattr(catalog, '_observation_encoder', None)
    if cached is None:
        from arkguardrl import structured as layout
        bank, _, _ = catalog.definitions()
        air_column = (layout.ENTITY_PREFIX + len(layout.KINDS) + len(layout.AREAS)
                      + layout.ENTITY_NUMBERS.index('can_hit_fly'))
        ranges = [catalog.attack_range({'kind': kind, 'id': identifier}) if kind == 'chess' else ()
                  for kind, identifier in bank['keys']]
        cached = kernel().ObservationEncoder(ranges, [bool(row[air_column]) for row in bank['entities']])
        catalog._observation_encoder = cached
    return cached


def number(value):
    # Rust checks finiteness after extraction. Preserve the public numeric type rule.
    if type(value) not in (int, float):
        raise ValueError('Structured features require finite public numbers')
    return value
