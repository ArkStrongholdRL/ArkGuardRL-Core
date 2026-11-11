"""Export bounded native battle frames and a standalone canvas replay.

An action file contains {"engine_seed": ..., "actions": [...]} and optionally
"core". Without it, the existing buy/deploy benchmark supplies a diagnostic
policy. Outcome is recorded explicitly, including defeats.
"""
import json
from pathlib import Path

from arkguardrl_core.native_env import load_native
from arkguardrl_core.native_gym import ArkGuardNativeEnv
from benchmarks.native_session import ending, pilot


def load_actions(path, core, max_bytes, *, include_config=False):
    if path.stat().st_size > max_bytes:
        raise ValueError('Action input exceeds replay byte limit')
    if path.suffix != '.jsonl':
        payload = json.loads(path.read_text())
        if payload.get('core', core) != core:
            raise ValueError('Action file core differs from --core')
        result = payload['engine_seed'], payload['actions']
        return (*result, payload.get('diy', {}), payload.get('setup_revision', 0)) if include_config else result
    header = None
    footer = None
    actions = []
    setup_revision = 0
    with path.open() as stream:
        for line in stream:
            row = json.loads(line)
            record = row.get('record', row)
            if header is None:
                header = record
                if header.get('kind') != 'header' or header.get('schema') not in (
                        'arkguardrl/native-battle-replay-v1', 'arkguardrl/native-trajectory-v1',
                        'stronghold-human-seat-trajectory/v1'):
                    raise ValueError('Unsupported trajectory header')
                if header.get('core', core) != core:
                    raise ValueError('Trajectory core differs from --core')
                if header.get('config', {}).get('difficulty', 'ABYSS') != 'ABYSS':
                    raise ValueError('Trajectory must use Desperation')
                if header['schema'] == 'stronghold-human-seat-trajectory/v1':
                    if header.get('roomMode') != 'solo' or header.get('difficulty') != 'ABYSS':
                        raise ValueError('Human trajectory must use solo Desperation')
            elif record.get('kind') == 'footer':
                footer = record
            elif record.get('kind') == 'action' and 'action' in record:
                action = record['action']
                if header['schema'] == 'stronghold-human-seat-trajectory/v1':
                    if action['t'] == 'room.rerollSetup' and record.get('accepted') and not actions:
                        if action.get('setupRevision') != setup_revision:
                            raise ValueError('Nonsequential setup reroll')
                        setup_revision += 1
                        continue
                    if record.get('accepted') is False:
                        continue
                    # Request correlation belongs to the live transport.
                    action = {key: value for key, value in action.items() if key != 'rid'}
                if action['t'] in ('g.infoReady', 'g.unitStats'):
                    continue
                if action['t'] == 'g.band':
                    if action['bandId'] != core:
                        raise ValueError('Recorded band differs from --core')
                    continue
                actions.append(action)
    if header is None or not actions:
        raise ValueError('Trajectory omitted its header or actions')
    if header['schema'] == 'stronghold-human-seat-trajectory/v1':
        if not footer or footer.get('complete') is not True:
            raise ValueError('Human trajectory did not complete its recording')
        engine_seed = header.get('seed')
        if type(engine_seed) is not int:
            raise ValueError('Human trajectory omitted its original seed')
    else:
        engine_seed = (header['engine_seed'] if 'engine_seed' in header
                       else header['config']['seed'])
    return (engine_seed, actions, header.get('diy', {}), header.get('setup_revision', setup_revision)) if include_config else (engine_seed, actions)


def record(session, engine_seed, actions, core, path, interval, max_bytes, diy=None, setup_revision=0):
    total_bytes = frames = 0
    session.reset(engine_seed)
    with path.open('xb') as stream:
        def append(row):
            nonlocal total_bytes
            payload = (json.dumps(row, ensure_ascii=False, separators=(',', ':'),
                                  allow_nan=False) + '\n').encode('utf-8')
            if total_bytes + len(payload) > max_bytes:
                raise RuntimeError('Replay byte limit reached before completion')
            stream.write(payload)
            total_bytes += len(payload)

        append(dict(kind='header', schema='arkguardrl/native-battle-replay-v1',
                    core=core, engine_seed=engine_seed, diy=diy or {}, setup_revision=setup_revision, tick_rate=30,
                    sample_ticks=interval, source='explicit-actions'))
        append(dict(kind='state', state=json.loads(session.observation())))
        for index, action in enumerate(actions):
            receipt = json.loads(session.step(json.dumps(action), 0))
            if not receipt['accepted']:
                raise RuntimeError(f'Rejected replay action {index}: {receipt}')
            append(dict(kind='action', index=index, action=action, receipt=receipt))
            while receipt['budgetExhausted']:
                frame = json.loads(session.replay_frame())
                if frame is None:
                    raise RuntimeError('Suspended combat omitted its replay frame')
                append(dict(kind='frame', **frame))
                frames += 1
                receipt = json.loads(session.advance(interval))
            append(dict(kind='state', state=json.loads(session.observation())))
        state = json.loads(session.observation())
        complete = state['personalEnded']
        summary = dict(kind='footer', status='complete' if complete else 'incomplete',
                       frames=frames, round=state['round'], outcome=state['outcome'],
                       diagnostics=json.loads(session.diagnostics()))
        append(summary)
    if not complete:
        raise RuntimeError('Action sequence ended before the episode completed')
    return summary


def run(*, data_root, native_library, output, core="band_chen", seed=42,
        action_file=None, require_victory=False, sample_ticks=3, max_bytes=128 << 20,
        html_output=None):
    output = Path(output)
    action_file = Path(action_file) if action_file is not None else None
    html_output = Path(html_output) if html_output is not None else None
    if core not in ("band_chen", "band_amiya", "band_sarkazb", "band_sciurus"):
        raise ValueError("Unsupported core")
    if not 1 <= sample_ticks <= 30 or max_bytes <= 0:
        raise ValueError('sample_ticks must be 1..30 and max_bytes must be positive')
    if output.exists() or (html_output and html_output.exists()):
        raise ValueError('Replay outputs must use new paths')
    expected = None
    diy = {}
    setup_revision = 0
    if action_file:
        engine_seed, actions, diy, setup_revision = load_actions(action_file, core, max_bytes, include_config=True)
    else:
        with ArkGuardNativeEnv(data_root, core=core,
                               native_library=native_library) as env:
            engine_seed, actions, expected = pilot(env, seed, True)
    native = load_native(native_library)
    catalog = native.NativeCatalog(data_root, 64 << 20)
    session = native.NativeSolo(catalog, core, diy_json=json.dumps(diy), setup_revision=setup_revision)
    try:
        summary = record(session, engine_seed, actions, core, output,
                         sample_ticks, max_bytes, diy, setup_revision)
        if expected is not None and ending(json.loads(session.observation())) != expected:
            raise RuntimeError('Frame capture changed the public terminal result')
    finally:
        session.close()
    if html_output:
        records = [json.loads(line) for line in output.read_text().splitlines()]
        template = Path(__file__).with_suffix('.html').read_text()
        data = json.dumps(records, ensure_ascii=False).replace('<', '\\u003c')
        with html_output.open('x') as handle:
            handle.write(template.replace('/*REPLAY_DATA*/ null', data))
    print(json.dumps(summary, ensure_ascii=False))
    if require_victory and (summary['outcome'] or {}).get('victory') is not True:
        raise RuntimeError('Replay completed with a defeat; victory acceptance failed')

    return summary
