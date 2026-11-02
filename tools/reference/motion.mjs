// Drive upstream movement functions directly. Export private functions in memory;
// the external checkout is never modified and this adapter is development-only.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const source=fs.readFileSync(path.join(root,'server/sim/ai.js'),'utf8').replace(/from\s+(['"])(\.\.?\/[^'"]+)\1/g,(_,q,p)=>`from ${q}${pathToFileURL(path.resolve(root,'server/sim',p))}${q}`)+'\nexport {advanceRoute,moveAttracted};';
const ai=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const fear=await imp('server/sim/fear.js');const{Grid}=await imp('server/sim/grid.js');const{BattleStatus}=await imp('server/sim/battle/status.js');const{Unit}=await imp('server/sim/units.js');const{createRng}=await imp('server/sim/rng.js');
const stages=JSON.parse(fs.readFileSync(path.join(root,'data/stages.json'),'utf8'));const stageIds=Object.keys(stages);const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<112;i++){
 const raw=stages[stageIds[i%stageIds.length]];let stage={...raw,legend:raw.tiles??raw.legend};if(i>=104){const rows=Array.from({length:19},()=>Array(21).fill('r'));rows[10][6]='I';rows[10][9]='O';rows[10][12]='E';stage={rows:rows.map(r=>r.join('')),legend:{}};}const rect={r0:0,r1:18,c0:0,c1:20},g=new Grid(stage,rect);
 const tiles=g.tiles.map((t,k)=>[t,k]).filter(([t,k])=>t.pass==='ALL'&&g.walkable((k/21)|0,k%21,true)).map(([,k])=>[(k/21)|0,k%21]);if(tiles.length<3)throw Error('fixture map lacks walkable tiles');
 const pick=j=>tiles[(i*7+j*13)%tiles.length];const start=i>=104?[10,5]:pick(0),dest=i>=104?[10,6]:pick(1),middle=i>=104?[10,5]:pick(2);const fly=i%3===0;const pos=[start[0]+(i%2?.25:0),start[1]+(i%2?-.25:0)];
 const u=new Unit({id:1,side:'enemy',kind:'enemy',x:pos[1],y:pos[0],motion:fly?'FLY':'WALK',base:{moveSpeed:i>=104?30:2+i%4,atk:0}});u.deployed=true;u.spawnY=pos[0];u.spawnX=pos[1];const src=new Unit({id:2,side:'ally',x:pos[1]-2,y:pos[0]-1});src.deployed=true;
 const route={checkpoints:[{type:'WAIT',time:.2},{type:'MOVE',pos:middle},{type:'DISAPPEAR'},{type:'WAIT',time:.3},{type:'APPEAR',pos:middle}],end:dest};u.route={legs:ai.compileRoute(route,rect),legIdx:0,pts:null,ptIdx:0,waitLeft:null};
 const trace=[];const b={grid:g,rect,units:[u,src],time:0,rng:createRng(i+1),_hooks:{},_safe:f=>f(),_ev(){},_cutAttackStand(){trace.push(['cut']);},_setHidden(e,on){e.hidden=on;trace.push(['hidden',on]);},_checkBlock(){return false;},leak(e){e.alive=false;e.deployed=false;trace.push(['leak']);},_unblock(){},releaseBlocked(){}};
 for(const key of['addBuff','removeBuff','_removeBuffAt','_tickBuffs','_applyValuedStatus','applyStatus','resistOf','_setAttractPoint'])b[key]=BattleStatus.prototype[key];
 const ticks=i>=104?360:160;const states=[];for(let tick=0;tick<ticks;tick++){
  b.time=tick/30;
  if(tick===2)b.applyStatus(u,'fear',{duration:.8,source:i%5===0?u:src});
  if(tick===8)b.applyStatus(u,'attract',{duration:1.5,point:[dest[0],dest[1]]});
  if(tick===20){u.x+=.3;u.y-=.2;}
  if(tick===25)g.setObstacle(...middle,true);
  if(tick===45)g.setObstacle(...middle,false);
  if(tick===35)b.applyStatus(u,'fear',{duration:.4,source:src});
  b._tickBuffs(1/30);
  if(u.alive){if(u.s.flags.fear&&!u.hidden)fear.moveFeared(b,u,1/30);else{if(u.mem.fearMove)fear.endFear(u);if(u.s.flags.attract){if(tick>=50&&tick<55)u.moving=false;else ai.moveAttracted(b,u,1/30);}else ai.advanceRoute(b,u,1/30,u.route,tick>=50&&tick<55);}}
  const m=u.mem.fearMove;const a=u.buffs.find(x=>x.status==='attract')?.data.attract;
  states.push({pos:[bits(u.y),bits(u.x)],moving:u.moving,alive:u.alive,hidden:u.hidden,rng:b.rng.state(),leg:u.route.legIdx,wait:u.route.waitLeft==null?null:bits(u.route.waitLeft),distance:bits(ai.remainingDistance(b,u)),fear:m?{seq:m.seq,tiles:[...m.tiles],point:m.i,key:m.k??null,goal:m.goal?[bits(m.goal.y),bits(m.goal.x)]:null,points:m.pts?.map(p=>[bits(p.y),bits(p.x)])??null}:null,attract:a?{point:a.i,points:a.pts?.map(p=>[bits(p.y),bits(p.x)])??null}:null});
 }
 process.stdout.write(JSON.stringify({i,stage:stageIds[i%stageIds.length],synthetic:i>=104?stage:null,ticks,pos,fly,route,middle,dest,states,trace})+'\n');
}
