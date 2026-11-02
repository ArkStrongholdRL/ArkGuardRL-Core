// Emit independent upstream results; no game data or match execution needed.
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const root = path.resolve(process.argv[2]);
const imp = p => import(pathToFileURL(path.join(root, p)).href);
const {createRng, deriveSeed} = await imp('server/sim/rng.js');
const math = await imp('server/sim/detmath.js');
const {VirtualScheduler} = await imp('server/match/scheduler.js');
const bits = x => { const b = Buffer.alloc(8); b.writeDoubleBE(x); return b.toString('hex'); };
const out = value => process.stdout.write(JSON.stringify(value) + '\n');
for (const seed of [0, 1, 42, 0xffffffff, 0x80000000]) {
  const rng = createRng(seed);
  out({kind:'rng', seed, values:Array.from({length:1024}, () => rng()), state:rng.state()});
  for (const salt of ['setup','shop','waves','draft','bots','meta','盟约😀'])
    out({kind:'derive', seed, salt, value:deriveSeed(seed,salt)});
}
const random = createRng(123);
for (const seed of [1, 42, 0x928d860b]) {
  const rng = createRng(seed);
  const first = rng();
  const cloned = rng.clone();
  const values = [rng.int(0), rng.pick([]) ?? null, rng.weighted([0, -1], x => x) ?? null];
  const shuffled = rng.shuffle([0,1,2,3,4,5,6,7]);
  const weights = [-1,0,1,2,3];
  const weighted = Array.from({length:32}, () => rng.weighted(weights, x => x));
  out({kind:'helpers', seed, first:bits(first), clone:bits(cloned()), values, shuffled, weighted, state:rng.state()});
}
const inputs = [-0, 0, 1e-300, -1e-300, 0.5, -0.5, 1.5, -2.5, Math.PI/4, Math.PI/2, Math.PI, -Math.PI, 1e6, 1e100];
for (let i=0;i<2048;i++) inputs.push((random()-.5)*1e6);
for (const x of inputs) {
  out({kind:'math', x:bits(x), sin:bits(math.sin(x)), cos:bits(math.cos(x)), round:bits(math.roundHalfEven(x))});
  for (const y of [-0, 0, 1, -1, x/2])
    out({kind:'pair', x:bits(x), y:bits(y), hypot:bits(math.hypot(x,y)), atan2:bits(math.atan2(y,x))});
}
for (const x of [-0,0,-2,-.1,.1,.9,1.01,2,100])
  for (const n of [-32,-3,-1,0,1,2,3,32,100])
    out({kind:'power', x:bits(x), n, value:bits(math.powi(x,n))});
const s=new VirtualScheduler({start:0});const events=[];
let handle=s.setInterval(()=>{events.push(['repeat',s.now()]);if(s.now()===4)s.clearInterval(handle);else s.setTimeout(()=>events.push(['callback',s.now()]),2);},2);
s.setTimeout(()=>events.push(['once',s.now()]),2);s.runAll({maxSteps:10});
out({kind:'scheduler',events});
