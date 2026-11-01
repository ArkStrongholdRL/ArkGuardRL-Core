// Continuous skill + search + real updateAlly/performAttack/resolveDamage integration.
// Real geometry, range index and skill conditions; movement is a phase fixture.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{BattleQueries}=await imp('server/sim/battle/queries.js');const{ProjectileSystem}=await imp('server/sim/projectiles.js');const{heal}=await imp('server/sim/attack/heal.js');const{Unit}=await imp('server/sim/units.js');const{SkillRuntime}=await imp('server/sim/skills.js');const ai=await imp('server/sim/ai.js');const{resolveDamage}=await imp('server/sim/attack/hit.js');const{makeDamageInfo}=await imp('server/sim/attack/helpers.js');const{canTargetEnemy}=await imp('server/sim/targeting.js');const{createRng}=await imp('server/sim/rng.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<24;i++){
 const d={i,kind:['instant','duration','ammo'][i%3],rule:i%4===0?'SP_FULL':'DEFAULT',attack:i%3===0,hits:1+i%2,dead:i%5===0,stop:i%7===0,store:i%11===0,stun:i%13===0,ranged:i%2===1,shooterDeath:i===23,healer:i%6===4};
 const u=new Unit({id:1,side:'ally',kind:'op',x:4,y:10,base:{maxHp:1000,atk:80,bat:.2}});u.deployed=true;u.rangeKeys=[216];u.baseRangeKeys=[216];u.profile={attack:d.ranged?'ranged':'melee',dmgType:d.healer?'heal':'phys',heal:d.healer?{mode:'single'}:null,atkScale:1,hits:d.hits,projectile:d.ranged?'arrow':'none',storeEnergy:d.store?(b,a)=>{trace.push(['store']);if(stored>=2)return false;stored++;return true;}:null};
 const e=new Unit({id:2,side:d.healer?'ally':'enemy',kind:'op',base:{maxHp:d.dead?500:100000,def:20}});e.deployed=true;if(d.healer)e.hp=100;e.spawnSeq=1;e.x=6;e.y=10;e.tileR=10;e.tileC=6;let stored=0;const trace=[];
 const baseBuff={key:'control',stacks:1,flags:{},mods:{},shield:0};u.buffs=[baseBuff];
 const b={time:0,dt:1/30,_attackSeq:0,_hooks:{skillStart:true,skillEnd:true,ammoUsed:true},allyUnits:d.healer?[u,e]:[u],enemies:d.healer?[]:[e],_eb:Array.from({length:399},()=>[]),_et:Array.from({length:399},()=>[]),_ebUsed:[],_etUsed:[],rng:createRng(42),kind:'normal',rangeChanged:()=>false,_refreshRange(){},remainingDistance:()=>5,
 enemiesInKeys:BattleQueries.prototype.enemiesInKeys,enemiesOnTiles:BattleQueries.prototype.enemiesOnTiles,blockedTargets:()=>[],_allyTargets:null,injuredAlliesInKeys:BattleQueries.prototype.injuredAlliesInKeys,
 addBuff(a,desc){a.buffs.push({...desc,stacks:1,shield:0});a.markDirty();},removeBuff(a,key){a.buffs=a.buffs.filter(b=>b.key!==key);a.markDirty();},_ev(){},_pp(){return null;},_safe:fn=>fn(),
 emit(name,c){trace.push([name]);},_skills:{onDamaged(t){t.skill?.onDamaged();}},kill(t){t.hp=0;t.alive=false;t.deployed=false;trace.push(['kill']);},
 heal(source,target,amount){return heal(this,source,target,amount);},dealDamage(source,target,request){return resolveDamage(this,{source,credit:source,target,dmg:makeDamageInfo(request)});},isAllyTarget:()=>false};b.projectiles=new ProjectileSystem(b);b.addProjectile=p=>b.projectiles.add(p);
 const spec={kind:d.kind,duration:.3,ammo:2,spCost:2,initSp:0,charges:1,spType:'attack',trigger:d.rule,attack:d.attack?{atkScale:1.5}:null,mods:{atkPct:.5},onStart(){trace.push(['start']);},onEnd(){trace.push(['end']);}};
 const skill=new SkillRuntime(b,u,{spCost:2,spType:'attack',skillType:'MANUAL'},spec);u.skill=skill;BattleQueries.prototype._buildEnemyIndex.call(b);skill.reset();const move=inside=>{e.x=inside?6:8;e.tileC=inside?6:8;BattleQueries.prototype._buildEnemyIndex.call(b);};
 const states=[];for(let tick=0;tick<240;tick++){
  b.time=tick/30;if(d.shooterDeath&&tick===2){u.alive=false;u.deployed=false;u.removed=true;skill.end('death');}if(d.healer)e.hp=Math.max(1,e.hp-3);move(tick%17<12);baseBuff.flags.stun=d.stun&&tick>=30&&tick<45;u.markDirty();
  ai.noteAttackCommit(b,b.dt);move((tick+1)%17<12);
  if(u.alive&&u.deployed)skill.tick(b.dt);ai.updateAlly(b,u,b.dt);b.projectiles.update(b.dt);if(d.stop&&tick===100)skill.end('stopped');
  states.push({hp:bits(e.hp),alive:e.alive,flights:b.projectiles.list.length,cd:bits(u.atkCd),had:u.trait.hadTarget??null,attacks:u.stats.attacks,sp:bits(skill.sp),charges:bits(skill.charges),active:skill.active,pending:skill.pending,time:bits(skill.timeLeft),ammo:bits(skill.ammoLeft),last:bits(u.lastAttackAt),atk:bits(u.s.atk)});
 }
 process.stdout.write(JSON.stringify({scenario:d,states,trace})+'\n');
}
