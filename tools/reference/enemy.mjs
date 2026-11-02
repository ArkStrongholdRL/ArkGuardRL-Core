// Actual updateEnemy + blocking + buff/HP/projectile systems, no match assets.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{BattleHooks}=await imp('server/sim/battle/hooks.js');const ai=await imp('server/sim/ai.js');const{Unit}=await imp('server/sim/units.js');const{Grid}=await imp('server/sim/grid.js');const{BattleStatus}=await imp('server/sim/battle/status.js');const{BattleBlocking}=await imp('server/sim/battle/blocking.js');const{BattleQueries}=await imp('server/sim/battle/queries.js');const{ProjectileSystem}=await imp('server/sim/projectiles.js');const{resolveDamage}=await imp('server/sim/attack/hit.js');const{makeDamageInfo}=await imp('server/sim/attack/helpers.js');const{heal}=await imp('server/sim/attack/heal.js');const{createRng}=await imp('server/sim/rng.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<144;i++){
 const cfg={i,mode:['phys','arts','true','heal','none'][i%5],melee:i%3===0,moves:i%7===0,deferred:i%11===0,blockFree:i%13===0,clip:i%4===0?null:{dur:[.4,.8,1.2][i%3],hit:i%7===0?null:.3},maxTargets:1+i%3,bat:[.4,.8,1][i%3],skip:i%17===0,keep:i%5===0,cancel:i%23===0,interrupt:i%29===0};
 const e=new Unit({id:1,side:'enemy',kind:'enemy',x:8,y:10,base:{maxHp:100000,atk:40,def:10,bat:cfg.bat,rangeRadius:3,moveSpeed:1}});e.deployed=true;e.def={dmgType:cfg.mode,applyWay:cfg.melee?'MELEE':'RANGED',attackMoves:cfg.moves,attackAnim:cfg.clip};e.profile={blockFree:cfg.blockFree,maxTargets:cfg.maxTargets,deferHit:cfg.deferred,shot:'shell',atkScale:1.2,canTarget:a=>!cfg.skip||a.id!==3};e.spawnY=10;e.spawnX=8;e.route={legs:ai.compileRoute({checkpoints:[{type:'WAIT',time:.1}],end:[10,4]}),legIdx:0,pts:null,ptIdx:0,waitLeft:null};
 const allies=[[10,6],[10,7],[11,7]].map((p,j)=>{const a=new Unit({id:j+2,side:'ally',kind:'op',x:p[1],y:p[0],base:{maxHp:i%19===0&&j===0?80:100000,blockCnt:2}});a.deployed=true;a.ground=true;a.deploySeq=j+1;a.aggroSeq=j+1;return a;});
 const partner=new Unit({id:5,side:'enemy',kind:'enemy',x:8,y:10,base:{maxHp:30000}});partner.hp=500;partner.deployed=true;
 const grid=new Grid({rows:Array(19).fill('r'.repeat(21)),legend:{}},{r0:0,r1:18,c0:0,c1:20});const trace=[];const b={units:[e,...allies,partner],allyUnits:allies,enemies:[e,partner],grid,time:0,rng:createRng(i+1),kind:'normal',rect:grid.rect,_attackSeq:0,_occ:Array(399).fill(null),_hooks:{enemyAttackStart:true,palsyTrigger:true,beforeAttack:true,attack:true},_safe:f=>f(),_pp:()=>null,_skills:{onDamaged(){}},_ev(ev){if(ev[0]==='atk')trace.push(ev);},fx(name){if(name==='palsy')trace.push(['palsy']);},_cutAttackStand(){},_setHidden(u,on){u.hidden=on;},leak(u){u.alive=false;u.deployed=false;trace.push(['leak']);},kill(u){u.hp=0;u.alive=false;u.deployed=false;trace.push(['kill',u.id]);},emit(name,c){
 if(name==='enemyAttackStart'){trace.push(['start',c.targets.map(u=>u.id)]);if(cfg.interrupt)b.addBuff(e,{key:'interrupt',duration:.2,flags:{stun:true}});}
 if(name==='palsyTrigger'){trace.push(['palsyTrigger',c.buff.stacks]);c.keep=cfg.keep;}
 if(name==='beforeAttack'){trace.push(['before',c.targets.map(u=>u.id)]);if(cfg.cancel)c.targets=[];}
 if(name==='attack'){trace.push(['attack',c.targets.map(u=>u.id),[...c.targetDeployments].map(([u,s])=>[u.id,s])]);if(i%13===0){e.base.atk+=1;e.markDirty();}}
 return c;},dealDamage(source,target,req){trace.push(['hit',source.id,target.id,bits(req.amount),req.type,req.attackId,!!req.isProjectile]);return resolveDamage(this,{source,credit:source,target,dmg:makeDamageInfo(req)});},heal(source,target,amount){return heal(this,source,target,amount);}};
 for(const a of allies)b._occ[a.tileR*21+a.tileC]=a;
 for(const key of['addBuff','removeBuff','_removeBuffAt','_tickBuffs','_applyValuedStatus','applyStatus','resistOf','_setAttractPoint'])b[key]=BattleStatus.prototype[key];
 for(const key of['_checkBlock','_blockerFor','_unblock','releaseBlocked'])b[key]=BattleBlocking.prototype[key];b._stealthSwitch=()=>{};
 b.alliesInRadius=BattleQueries.prototype.alliesInRadius;b.enemiesInRadius=BattleQueries.prototype.enemiesInRadius;b.projectiles=new ProjectileSystem(b);b.addProjectile=p=>b.projectiles.add(p);
 const dispatch=b.emit;b._hooks=Object.create(null);b._emitDepth=0;b._frameName=[];b._frameOwner=[];b._frameCtx=[];b._handlerError=(label,owner,error)=>{throw error;};b.on=BattleHooks.prototype.on;b.off=BattleHooks.prototype.off;b.emit=BattleHooks.prototype.emit;for(const name of ['enemyAttackStart','palsyTrigger','beforeAttack','attack'])b.on(name,c=>dispatch.call(b,name,c),{owner:e});
 b.applyStatus(e,'palsy',{value:i%3+1,duration:2});const states=[];
 for(let tick=0;tick<180;tick++){
  b.time=tick/30;
  if(tick===3){allies[0].x=15;allies[0].y=15;}
  if(tick===30){allies[0].x=6;allies[0].y=10;}
  if(tick===20)b.applyStatus(e,'stun',{duration:.2});
  if(tick===45)b.applyStatus(allies[1],'sleep',{duration:.2});
  if(tick===65)b.applyStatus(e,'disarm',{duration:.25});
  if(tick===90)b.applyStatus(e,'fear',{duration:.25,source:allies[0]});
  if(tick===110)b.applyStatus(e,'attract',{duration:.25,point:[10,8]});
  if(tick===100)e.unbalanceUntil=b.time+.3;
  if(tick===104)b.applyStatus(e,'levitate',{duration:.1});
  if(tick===80){e.profile.atkScale=1.5;e.base.atk+=3;e.markDirty();}
  if(tick===130&&i%9===0){e.alive=false;e.deployed=false;}
  e.mem.carrier=tick>=70&&tick<75?partner:null;e.hidden=tick>=50&&tick<55;e.pauseUntil=tick>=60&&tick<64?3:-Infinity;
  b._tickBuffs(1/30);ai.updateEnemy(b,e,1/30);b.projectiles.update(1/30);
  states.push({hp:b.units.map(u=>bits(u.hp)),alive:b.units.map(u=>u.alive),pos:[bits(e.y),bits(e.x)],moving:e.moving,cd:bits(e.atkCd),swing:!!e.swing,targets:e.swingTargets?.map(u=>u.id)??null,stand:bits(e.atkStandUntil??-Infinity),unbalance:bits(e.unbalanceUntil??-Infinity),blocked:e.blockedBy?.id??null,attacks:e.stats.attacks,last:bits(e.lastAttackAt),flights:b.projectiles.list.length,rng:b.rng.state(),palsy:e.findBuff('palsy')?.stacks??0});
 }
 process.stdout.write(JSON.stringify({cfg,states,trace})+'\n');
}
