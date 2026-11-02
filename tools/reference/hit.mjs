// HP-damage pipeline oracle; no match or data loading.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{resolveDamage}=await imp('server/sim/attack/hit.js');const{createRng}=await imp('server/sim/rng.js');const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<1024;i++){
 const d={i,type:['phys','arts','true','elemental'][i%4],action:Math.floor(i/4)%8,calculation:['formula','precomputed','resolved'][Math.floor(i/32)%3],source:i%13!==0,enemy:i%7===0,sleep:i%17===0,hitSleep:i%11===0,liftoff:i%19===0,hidden:i%37===0,fly:i%23===0,ignoreSelect:i%29===0,boss:i%41===0,amount:i%43===0?300000:i%211+50,shield:i%9===0?300:20,dodge:i%5===0?.8:0,mode:['ignore','normal','uncancellable'][Math.floor(i/96)%3]};
 const source=new Unit({id:1,side:d.enemy?'enemy':'ally',base:{maxHp:1000}});source.deployed=true;source.motion=d.fly?'FLY':'WALK';source.profile={hitSleep:d.hitSleep};
 const target=new Unit({id:2,side:'ally',base:{maxHp:1000,def:60,res:30}});target.deployed=true;target.hidden=d.hidden;target.isBoss=d.boss;
 const baseBuff={stacks:1,mods:{artsAttackAddition:7},flags:{sleep:d.sleep,liftoff:d.liftoff},shield:0};const shieldBuff={stacks:1,mods:{dmgTakenMul:1.2,dodgePhys:d.dodge,dodgeArts:d.dodge},flags:null,shield:d.shield,shieldHits:0,seq:1,key:'shield'};target.buffs=[baseBuff,shieldBuff];
 const trace=[];const rng=createRng(i+1);const phases=['hit','damageCalculate','damageCalculateTarget','damageOutput','damageBeforeApply','damageApplying','hpDamage'];const health=['fatal','hpChanged','modifierApplied','damageApplied','damaged'];
 const battle={time:3,kind:'boss',rng,_hooks:Object.fromEntries([...phases,...health,'block','dodge'].map(k=>[k,true])),_pp(){return null;},_safe(fn){fn();},_internalError(k,e){throw e;},_ev(){},_removeBuffAt(t,j){t.buffs.splice(j,1);},fx(k,e){if(k==='hitCap')trace.push(['hitCap',bits(e.n)]);},
 emit(stage,e){if(phases.includes(stage)){trace.push([stage,bits(e.amount??e.dmg.amount)]);
 if(stage==='hit'){if(d.action===1)e.dmg.cancel=true;if(d.action===3){baseBuff.flags.invulnerable=true;target.markDirty();}if(d.action===4){baseBuff.flags.liftoff=true;target.markDirty();}}
 if(stage==='damageCalculateTarget'){if(d.action===2)target.alive=false;if(d.action===5){target.base.def=120;target.markDirty();}}
 if(stage==='damageOutput'&&d.action===6){e.amount*=.5;e.dmg.dodgeMode='ignore';}if(stage==='hpDamage'&&d.action===7)e.amount=3;
 }else if(stage==='block'||stage==='dodge')trace.push([stage,e.cancelled]);else trace.push([stage,bits(e.amount)]);},kill(t){trace.push(['kill']);t.alive=false;t.deployed=false;}};
 const dmg={amount:d.amount,type:d.type,mul:1,defIgnoreFlat:0,defIgnorePct:0,resIgnoreFlat:0,resIgnorePct:0,calculation:d.calculation,ignoreSelect:d.ignoreSelect,dodgeMode:d.mode,silent:true,noSp:true};
 const result=resolveDamage(battle,{source:d.source?source:null,credit:d.source?source:null,target,dmg});
 process.stdout.write(JSON.stringify({scenario:d,result:bits(result),hp:bits(target.hp),alive:target.alive,deployed:target.deployed,shield:bits(shieldBuff.shield),taken:bits(target.stats.taken),damage:bits(source.stats.dmg),rng:rng.state(),trace})+'\n');
}
