"""Paired single-env benchmark using the original-runtime JSON bridge."""
import argparse
import sys, json, time, statistics
from pathlib import Path
repo=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--reference-root', required=True)
parser.add_argument('--native-library', required=True)
parser.add_argument('--out', type=Path, required=True)
args=parser.parse_args()
sys.path.insert(0,str(repo/'benchmarks'))
import native_session as b
import node_bridge as ref
reference_root=str(Path(args.reference_root).resolve())
native=b.load_native(args.native_library)
catalog=native.NativeCatalog(reference_root+'/data',64<<20)
def normalize(x):
    if isinstance(x,dict):return {k:normalize(v) for k,v in x.items()}
    if isinstance(x,list):return [normalize(v) for v in x]
    return 'solo' if x=='agent' else x

def node_run(oracle,cases,cycles,core):
    durations=[];steps=0
    for _ in range(cycles):
        for _,seed,actions,expected in cases:
            start=time.perf_counter()
            oracle.call(cmd='reset',seed=seed,difficulty='ABYSS')
            oracle.call(cmd='step',action={'t':'g.infoReady'})
            oracle.call(cmd='step',action={'t':'g.band','bandId':core})
            for action in actions:
                reply=oracle.call(cmd='step',action=action)
                if not reply.get('accepted'):raise RuntimeError(reply)
                state=reply['state'];steps+=1
            for _ in range(100):
                if state['gameEnded']:break
                state=oracle.call(cmd='advance',maxSteps=4096)['state']
            durations.append(time.perf_counter()-start)
            if not state['gameEnded']:raise RuntimeError('Not terminal')
            ref.compare_public(expected,normalize(b.ending(state)))
    seconds=sum(durations)
    return dict(seconds=seconds,episodes=len(durations),decisions=steps,episodes_per_second=len(durations)/seconds,decisions_per_second=steps/seconds)

rows=[]
for core in b.CORES:
    session=native.NativeSolo(catalog,core)
    with b.ArkGuardNativeEnv(reference_root+'/data',core=core,native_library=args.native_library,profile=True) as env, ref.Oracle(reference_root) as oracle:
        oracle.call(cmd='identity')
        for roster in (False,True):
            cases=[(seed,*b.pilot(env,seed,roster)) for seed in (42,99)]
            b.measure(env,session,cases,2,'native')
            node_run(oracle,cases,2,core)
            runs={'native':[],'node':[]}
            for repeat in range(3):
                # Alternate order to reduce systematic warmup/thermal bias.
                for backend in (('native','node') if repeat%2==0 else ('node','native')):
                    r=b.measure(env,session,cases,5,'native') if backend=='native' else node_run(oracle,cases,5,core)
                    runs[backend].append(r)
            row=dict(core=core,workload='buy_deploy' if roster else 'empty',runs=runs)
            for backend in runs:
                row[backend+'_episodes_per_second']=statistics.median(r['episodes_per_second'] for r in runs[backend])
            row['speedup']=row['native_episodes_per_second']/row['node_episodes_per_second']
            rows.append(row)
            print(json.dumps({k:v for k,v in row.items() if k!='runs'}),flush=True)
    session.close()
out=dict(note='Same engine seeds and actions, terminal public subset checked. Warm persistent Node Match JSON bridge vs native PyO3 JSON session; excludes startup and pilot, includes reset and public observations. No Gym encoding in either timed path.',results=rows)
args.out.parent.mkdir(parents=True,exist_ok=True)
args.out.write_text(json.dumps(out,indent=2)+'\n')
