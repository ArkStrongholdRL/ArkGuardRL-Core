// Real acquireTargets and noteAttackCommit with explicit range-query fixtures.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const ai=await imp('server/sim/ai.js');const{canTargetEnemy}=await imp('server/sim/targeting.js');
for(let i=0;i<512;i++){
 const d={i,heal:i%5===0,multi:i%3===0,hpAtMost:i%2?.5:0,healCount:i%4,extraTargets:i%3,maxTargets:i%5,fortress:i%7===0,blocked:i%4===0,all:i%11===0,rangeAoe:i%13===0,hitAll:i%17===0,empty:i%3===0,hasExtra:i%2===0,nearExtra:i%19===0,cooldown:i%23===0?1:1/30,had:i%29===0?false:null};
 const units=Array.from({length:9},(_,j)=>{const id=j+1;const u=new Unit({id,side:id===1||id===9?'ally':'enemy',kind:'op',tileR:10,tileC:4,base:{maxHp:100,blockCnt:3}});u.deployed=true;u.hp=id*10;u.spawnSeq=10-id;u.rangeKeys=[1];return u;});const u=units[0];u.buffs=[{stacks:1,mods:{maxTargets:d.extraTargets},flags:{}}];u.atkCd=d.cooldown;u.trait.hadTarget=d.had;u.skill={active:true,attackOverride:()=>null,targetingOverride:()=>null};if(d.blocked){u.blocking=[units[1]];units[1].blockedBy=u;}
 const p={heal:d.heal?{mode:d.multi?'multi':'single',count:d.healCount,hpAtMost:d.hpAtMost}:null,dmgType:d.heal?'heal':'phys',fortress:d.fortress,allInRange:d.all,rangeAoe:d.rangeAoe,hitAllBlocked:d.hitAll,maxTargets:d.maxTargets,priority:'lowestHp',canHitFly:true};u.profile=p;
 const fixture={enemies:d.empty?[]:[4,3,5],near:d.empty?[6,7]:[],extra:d.hasExtra&&!d.empty?[9]:[],nearExtra:d.nearExtra?[9]:[],heal:[2,3,4,5,6,7,8,9]};const map=ids=>ids.map(id=>units[id-1]);
 const b={allyUnits:[u],_allyTargets:d.hasExtra||d.nearExtra?new Set([9]):null,remainingDistance:e=>10-e.id,enemiesInKeys(keys){return map(keys[0]===1?fixture.enemies:fixture.near);},injuredAlliesInKeys:()=>map(fixture.heal),allyTargetsInKeys(keys){return map(keys[0]===1?fixture.extra:fixture.nearExtra);},blockedTargets(a,prof){return a.blocking.filter(e=>canTargetEnemy(a,e,prof));}};
 const state={};const selected=ai.acquireTargets(b,u,p,state).map(t=>t.id);ai.noteAttackCommit(b,1/30);const raw=b._attackCommit?.get(1)?.map(t=>t.id)??[];
 units[1].alive=false;units[2].buffs=[{stacks:1,mods:{},flags:{untargetable:true}}];units[2].markDirty();
 const fallback=raw.filter(id=>canTargetEnemy(u,units[id-1],p));
 process.stdout.write(JSON.stringify({scenario:d,fixture,selected,melee:state.fortressMelee??null,raw,fallback})+'\n');
}
