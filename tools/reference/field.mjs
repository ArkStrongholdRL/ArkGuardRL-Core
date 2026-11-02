// Concrete native field oracle: full upstream content, empty allied roster.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]),imp=p=>import(pathToFileURL(path.join(root,p)));
const{Battle}=await imp('server/sim/Battle.js'),content=await imp('server/sim/content/enemies.js');
const{kitStealth,kitRefraction}=await imp('server/sim/content/enemies/archetypes.js');
const data=Object.fromEntries(['config','chess','bonds','garrisons','items','bands','effects','choices','enemies','factions','waves','stages','bosses','tokens','backups'].map(k=>[k,JSON.parse(fs.readFileSync(path.join(root,'data',k+'.json'),'utf8'))]));
const keys=Object.keys(data.enemies).filter(k=>content.STATS_ONLY[k]||content.KITS[k]===kitStealth||content.KITS[k]===kitRefraction||k==='enemy_9011_acrefr'||/^enemy_(1196_msfyin|1198_msfshu|1200_msfjin|1202_msfzhi|1204_msfhu|1208_msfji|1210_msfden|1148_dssbr|1305_mhslim|2021_syfish|1229_darmy)(_2)?$/.test(k));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
const stage={rows:Array(19).fill('r'.repeat(21)),legend:{}};
for(let i=0;i<keys.length;i++){
 const key=keys[i],routes=[{start:[10,10],end:[10,0],checkpoints:[]},{start:[12,10],end:[9,0],steps:[{t:'WAIT',s:.2},{t:'DISAPPEAR'},{t:'WAIT',s:.1},{t:'APPEAR',p:[10,8]}]}];
 const spawns=[{enemyKey:key,time:0,count:2,interval:.2,mods:{hpMul:1.3,speedMul:2.3,supplyHpMul:1.5}},{enemyKey:'enemy_1007_slime',time:.1,routeIndex:1},{enemyKey:key,time:9,tag:'duck'}];
 const options={stage,seed:i+1,timeLimit:i%2?12:3,players:[{playerId:'p',units:[]}],spawns,routes};
 const b=new Battle({...options,data,content:'full',recordEvents:false});
 const report=()=>{const r=b.result(),p=r.perPlayer.p,pp={};for(const k of['killed','total','leaked','perfect','killedInTotal','leakedInTotal','resolved','creditedKills','coins'])pp[k]=p[k];return structuredClone({...Object.fromEntries(['time','reason','killed','total','resolved','errors'].map(k=>[k,r[k]])),perPlayer:{p:pp},...(r.unspawned?{unspawned:r.unspawned}:{})});};
 const view=()=>({time:bits(b.time),ticks:b.tickCount,finished:b.finished,reason:b.reason,pending:b._pending.length,sequence:b._spawnSeq,rng:b.rng.state(),units:b.enemies.filter(e=>e.alive).map(e=>{const s=e.s;return{id:e.id,key:e.defId,pos:[bits(e.y),bits(e.x)],hp:bits(e.hp),maxHp:bits(s.maxHp),atk:bits(s.atk),res:bits(s.res),cd:bits(e.atkCd),moving:e.moving,hidden:e.hidden,leg:e.route.legIdx,spawn:e.spawnSeq,deploy:e.deploySeq,element:['burn','neural','necrosis','apoptosis','erosion'].map(k=>bits(e.elem[k])),flags:Object.keys(s.flags).filter(k=>s.flags[k]).sort()};}),report:report()});
 const states=[],ops=[];
 for(let t=0;t<400&&!b.finished;t++){
  if(t===15){const target=b.enemies.find(e=>e.alive);if(target){const spec={amount:target.hp*.3,type:'true'};b.dealDamage(null,target,spec);ops.push({tick:t,op:'damage',id:target.id,spec});}}
  if(t===30){const target=b.enemies.find(e=>e.alive);if(target){const spec={amount:target.hp+100,type:'true'};b.dealDamage(null,target,spec);ops.push({tick:t,op:'damage',id:target.id,spec});}}
  if(t===35){const options={pos:[10,8],route:routes[0],bounty:{coins:3},mods:{speedMul:1.2},inTotal:i%3===0};b.spawnEnemy(key,options);ops.push({tick:t,op:'spawn',key,options});}
  if(t===40){const source=b.enemies.find(e=>e.alive&&e.defId===key),target=b.enemies.find(e=>e.alive&&e!==source);if(source&&target){for(let j=0;j<30;j++)b.dealDamage(source,target,{amount:0,type:'true',isAttack:true,attackId:700});ops.push({tick:t,op:'hitChain',source:source.id,id:target.id});}}
  b.step();states.push(view());
 }
 console.log(JSON.stringify({options,ops,states}));
}
