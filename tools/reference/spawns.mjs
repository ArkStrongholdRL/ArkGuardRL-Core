// No renderer: real headless Battle.step with content disabled to isolate the
// native wave/unit/tick boundary. Content abilities have separate parity tests.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);const{Battle}=await import(pathToFileURL(path.join(root,'server/sim/Battle.js')));
const stages=JSON.parse(fs.readFileSync(path.join(root,'data/stages.json'),'utf8'));
const enemies=JSON.parse(fs.readFileSync(path.join(root,'data/enemies.json'),'utf8'));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<40;i++){
 const stageId=Object.keys(stages)[i%13],keys=Object.keys(enemies);
 const spawns=Array.from({length:i===0?0:7},(_,j)=>({enemyKey:keys[(i*7+j)%keys.length],time:j%2?j*0.07:0,count:j===0?2:1,interval:.1,routeIndex:j%2,mods:{hpMul:1.2,speedMul:i%3?1.4:0.3},countInTotal:j%4!==0,tag:j===4?'part':null}));
 spawns.push({enemyKey:'enemy_1007_slime',time:20,count:2});
 if(i%5===0)spawns.length=0;
 const routes=[{start:[9,10],end:[9,i%2?9:2],checkpoints:[]},{start:[12,10],end:[9,2],steps:[{t:'WAIT',s:.17},{t:'DISAPPEAR'},{t:'WAIT',s:.1},{t:'MOVE',p:[10,9]}]}];
 const options={stageId,seed:i+1,timeLimit:i%3?8:2.5,players:[{playerId:'p',units:[]}],spawns,routes,enemyOverrides:i%2?{}:{enemy_1007_slime:{stats:{moveSpeed:2.4}}}};
 // Isolate terrain/path physics. Stage devices create real allied combatants
 // even with content:'none', and belong to their own upcoming content port.
 const b=new Battle({...options,stage:{...stages[stageId],devices:[]},data:{enemies,stages},content:'none',recordEvents:false,logger:{warn(){},error(){}}});
 const report=()=>{const r=b.result(),p=r.perPlayer.p,pp={};for(const k of ['killed','total','leaked','perfect','killedInTotal','leakedInTotal','resolved','creditedKills','coins'])pp[k]=p[k];return structuredClone({...Object.fromEntries(['time','reason','killed','total','resolved','errors'].map(k=>[k,r[k]])),perPlayer:{p:pp},...(r.unspawned?{unspawned:r.unspawned}:{})});};
 const view=()=>({time:bits(b.time),ticks:b.tickCount,finished:b.finished,reason:b.reason,pending:b._pending.length,sequence:b._spawnSeq,units:b.enemies.filter(e=>e.alive).map(e=>({id:e.id,key:e.defId,pos:[bits(e.y),bits(e.x)],hp:bits(e.hp),cd:bits(e.atkCd),hidden:e.hidden,moving:e.moving,leg:e.route.legIdx,spawn:e.spawnSeq,deploy:e.deploySeq})),report:report()});
 const states=[],ops=[];
 for(let t=0;t<280&&!b.finished;t++){
  if(t===6)for(const e of b.enemies)if(e.alive)e.hp*=.5;
  if(t===9){const def={pos:[9,9.8],route:routes[0],mods:{speedMul:.7}};const e=b.spawnEnemy('enemy_1007_slime',def);ops.push({tick:t,op:'spawn',key:e.defId,def});}
  if(t===30){const e=b.enemies.find(e=>e.alive&&!e.isBoss);if(e){b.kill(e);ops.push({tick:t,op:'kill',id:e.id});}}
  b.step();states.push(view());
 }
 console.log(JSON.stringify({options,ops,states}));
}
