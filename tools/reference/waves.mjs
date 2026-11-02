// Development-only oracle: real 0.3.3 template conversion and solo Battle billing.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const root = path.resolve(process.argv[2]);
const {spawnsFromTemplate} = await import(pathToFileURL(path.join(root,'server/sim/simdata.js')));
const {Battle} = await import(pathToFileURL(path.join(root,'server/sim/Battle.js')));
const bits = x => { const b=Buffer.alloc(8); b.writeDoubleBE(x); return b.toString('hex'); };
const raw=JSON.parse(fs.readFileSync(path.join(root,'data/waves.json'),'utf8'));
const catalogEnemies=JSON.parse(fs.readFileSync(path.join(root,'data/enemies.json'),'utf8'));
const queueView=b=>b._pending.map(p=>({time:bits(p.time),sequence:p.seq,key:p.enemyKey,counted:p.counted,inTotal:p.inTotal,precounted:!!p.precounted}));
for (const [key,tpl] of Object.entries(raw)) {
  const opts={mods:{hpMul:1.25,atkMul:0.7},tag:'wave'};
  const output=spawnsFromTemplate(tpl,opts);
  const b=new Battle({content:'none',recordEvents:false,spawns:output.spawns,data:{enemies:catalogEnemies},logger:{warn(){},error(){}}});
  const excluded=Object.keys(catalogEnemies).filter(k=>catalogEnemies[k].notCountInTotal);
  console.log(JSON.stringify({kind:'template',key,input:tpl,opts,output,queue:queueView(b),excluded}));
}
// Old research-shaped template; aliases and cumulative timings use upstream code.
const tpl={routes:[{m:'F',s:{r:9,c:10},e:[9,2],cp:[['DISAPPEAR',0,0],['WAIT',0,0,3],['MOVE',9,5]]}],waves:[
 {preDelay:0.2,postDelay:0.3,fragments:[{preDelay:0.4,actions:[{key:'enemy_a',n:3,int:0.1,pre:0.2,route:0}]},{preDelay:0.3,actions:[{key:'enemy_b',n:2,int:0.7}]}]},
 {preDelay:0.6,fragments:[{actions:[{enemyKey:'enemy_c',count:1,preDelay:0.2}]}]}]};
console.log(JSON.stringify({kind:'template',key:'research',input:tpl,opts:{},output:spawnsFromTemplate(tpl)}));
const enemies={enemy_a:{stats:{hp:400,atk:0,moveSpeed:1},lpr:2},enemy_b:{stats:{hp:500},notCountInTotal:true},enemy_c:{stats:{hp:600},lpr:3}};
const project = b => {
 const r=b.result(),p=r.perPlayer.p;
 const pp={}; for(const k of ['killed','total','leaked','perfect','killedInTotal','leakedInTotal','resolved','creditedKills','coins'])pp[k]=p[k];
 return structuredClone({...Object.fromEntries(['time','reason','killed','total','resolved','errors'].map(k=>[k,r[k]])),perPlayer:{p:pp},...(r.unspawned?{unspawned:r.unspawned}:{})});
};
for(let i=0;i<160;i++) {
 const spawns=[
  {enemyKey:'enemy_a',time:0.1*(i%3),count:3,interval:0.1,bounty:{coins:2}},
  {enemyKey:'enemy_b',time:0.1,count:2,interval:0.15,countInTotal:i%2===0},
  {enemyKey:'enemy_c',time:3,count:2,tag:i%3===0?'boss':i%3===1?'part':null},
  {enemyKey:'enemy_a',time:0.3,sourcePlayerId:'other',mods:{hpMul:1.2}},
 ];
 const b=new Battle({seed:1,content:'none',recordEvents:false,players:[{playerId:'p',units:[]}],spawns,data:{enemies},logger:{warn(){},error(){}}});
 const queue=queueView(b);
 const ops=[],states=[project(b)];
 for(let k=0;k<12;k++){
  const time=k*0.1;b.time=time;
  if(k===2){ const def={enemyKey:'enemy_a',tag:i%4===0?'part':null,countInTotal:i%7===0?false:undefined,inTotal:i%5===0,bounty:{coins:3},sourcePlayerId:'runtime'};
   b.spawnEnemy(def.enemyKey,def);ops.push({op:'spawn',time,def});states.push(project(b)); }
  b._processSpawns();ops.push({op:'process',time});states.push(project(b));
  if(k%3===i%3){const e=b.enemies.find(e=>e.alive);if(e){const op=k%2?'kill':'leak';b[op](e);ops.push({op,time,id:e.id});states.push(project(b));}}
 }
 b.time=1.2;b.forceEnd(i%2?'timeout':'forced');ops.push({op:i%2?'timeout':'forced',time:b.time});states.push(project(b));
 console.log(JSON.stringify({kind:'billing',enemies,spawns,queue,ops,states}));
}
