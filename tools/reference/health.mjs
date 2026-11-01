// HP and healing oracle; no match, assets or training.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{applyHpDecrease,loseHp}=await imp('server/sim/attack/health.js');const{heal}=await imp('server/sim/attack/heal.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<768;i++){
 const scenario={i,type:['phys','arts','true','elemental'][i%4],heal:i%3===0,loss:i%3===1,pool:i%11===0,boss:i%7===0,amount:i%17===0?300000:i%29===0?299999.1:(i*37)%230,self:i%13===0,targetAlly:i%2===0,fatal:i%4,mode:['heal','regen','restore'][Math.floor(i/3)%3],noHeal:i%5===0,healFree:i%7===0,noSp:i%9===0,through:i%11===0,hitCount:i%13===0,hitCountArts:i%17===0};
 const d=scenario;const source=new Unit({id:1,side:'ally',base:{maxHp:100,atk:100}});source.ownerId=10;source.deployed=true;
 const target=new Unit({id:2,side:d.targetAlly?'ally':'enemy',base:{maxHp:100,def:25,res:30}});target.hp=37;target.deployed=true;target.isBoss=d.boss;target.skill={};
 target.buffs=[{stacks:1,mods:{healingTakenMul:1.3},flags:{noHeal:d.noHeal,healFree:d.healFree,hitCount:d.hitCount,hitCountArts:d.hitCountArts},shield:0}];source.buffs=[{stacks:1,mods:{healingDealtMul:1.2},shield:0}];
 source.profile={healThrough:()=>d.through};const pool={hp:63,maxHp:100,damage(pid,a){this.hp=Math.max(0,this.hp-a);}};if(d.pool)target.bossPool=pool;
 const credit=d.self?target:source;const trace=[];const pp={damageDealt:0,bossDamage:0,healingDone:0};let shield=0;
 const stages=['fatal','hpChanged','modifierApplied','damageApplied','damaged','heal'];
 const battle={_safe(fn){fn();},time:5,kind:'boss',_hooks:Object.fromEntries(stages.map(k=>[k,true])),_pp:()=>pp,
 emit(stage,e){trace.push([stage,bits(e.amount)]);if(stage==='fatal'){if(d.fatal===1)e.prevented=true;if(d.fatal===2)target.hp=5;if(d.fatal===3)target.alive=false;}if(stage==='heal')e.amount*=.5;},
 _syncBossHp(t){t.hp=t.s.maxHp*Math.min(1,t.bossPool.hp/t.bossPool.maxHp);},_internalError(label,e){throw e;},fx(k,e){trace.push([k,bits(e.n)]);},_ev(){},
 _skills:{onDamaged(){trace.push(['skill']);}},kill(t,c){trace.push(['kill']);t.alive=false;t.deployed=false;},
 addBuff(t,b){shield=Math.min(100,shield+b.shield);},};target.findBuff=()=>shield?{shield}:null;
 const dmg={type:d.type,noSp:d.noSp,silent:false,defIgnoreFlat:0,defIgnorePct:0,resIgnoreFlat:0,resIgnorePct:0};
 let result;if(d.heal)result=heal(battle,d.self?target:source,target,d.amount,{mode:d.mode,overheal:true});else{const ctx={source,credit,target,dmg,onApplied(){trace.push(['applied']);}};result=d.loss?loseHp(battle,ctx,d.amount):applyHpDecrease(battle,ctx,d.amount);}
 process.stdout.write(JSON.stringify({scenario:d,result:bits(result),hp:bits(target.hp),alive:target.alive,deployed:target.deployed,taken:bits(target.stats.taken),damage:bits(credit.stats.dmg),healing:bits((d.self?target:source).stats.heal),last:bits(target.lastHitAt),pool:bits(pool.hp),pp:Object.fromEntries(Object.entries(pp).map(([k,v])=>[k,bits(v)])),shield:bits(shield),trace})+'\n');
}
