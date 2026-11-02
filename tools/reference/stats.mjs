// Numeric battle oracle: no matches or game data.
import path from 'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{aggregateMods,ADD_KEYS,MUL_KEYS}=await imp('server/sim/buffs.js');
const calc=await imp('server/sim/attack/calculation.js');const{createRng}=await imp('server/sim/rng.js');
const rng=createRng(74581);const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
const encoded=s=>({numbers:Object.fromEntries(Object.entries(s).filter(([,v])=>typeof v==='number').map(([k,v])=>[k,bits(v)])),flags:Object.keys(s.flags).filter(k=>s.flags[k]).sort()});
const out=x=>process.stdout.write(JSON.stringify(x)+'\n');
for(let i=0;i<768;i++){
 const base={maxHp:100+rng()*1000,atk:rng()*2000,def:rng()*1000,res:rng()*150,aspd:rng()*800-100,bat:rng()*4,blockCnt:rng()*5,moveSpeed:rng()*3,spRecovery:rng()*2,massLevel:rng()*5,hpRecoveryPerSec:rng(),tauntLevel:rng()*2};
 const buffs=[];for(let j=0;j<i%8;j++){
  const mods={};for(const k of ADD_KEYS)if(rng()<.35)mods[k]=rng()*2-1;
  for(const k of MUL_KEYS)if(rng()<.35)mods[k]=rng()*2;
  const flags={};for(const k of ['taunt','stun','freeze','sleep','levitate','float','groundbind'])if(rng()<.15)flags[k]=true;
  const permanent=rng()<.5;buffs.push({mods,flags,stacks:1+Math.floor(rng()*4),shield:rng()*10,persist:permanent,duration:permanent?Infinity:10});
 }
 // Exercise collapse, overflow fallbacks, unclamped additive mods and independent dodge.
 if(i%64===0)buffs.push({mods:{hpPct:-2,atkPct:-2,defPct:-2,batPct:-2,dodgePhys:-.1},flags:{},stacks:1,shield:0,duration:10});
 if(i%64===1)buffs.push({mods:{hpMul:1e100,atkMul:1e100,defMul:1e100},flags:{},stacks:4,shield:0,duration:10});
 const cm={hp:.5+rng()*2,atk:.5+rng()*2,def:.5+rng()*2};const floor=rng();
 const unit=new Unit({id:1,side:'enemy',kind:'enemy',base});unit.cultMul=cm;unit.mem.moveSpeedFloor=floor;unit.hp*=.37;unit.buffs=buffs;
 const s=unit.s;const first=encoded(s),hp=bits(unit.hp);
 const serialized=buffs.map(b=>({mods:b.mods,flags:b.flags,stacks:b.stacks,shield:b.shield,permanent:b.persist&&b.duration===Infinity}));
 unit.buffs=[];unit.markDirty();const second=encoded(unit.s);const hpAfter=bits(unit.hp);
 const target=new Unit({base:{maxHp:1,def:rng()*2000,res:rng()*150}}).s;
 const dmg={amount:rng()*3000,mul:rng()*3,defIgnoreFlat:rng()*100,defIgnorePct:rng()*2-.5,resIgnoreFlat:rng()*30,resIgnorePct:rng()*2-.5,calculation:i%2?'resolved':'formula'};
 const ep=rng()*150;const targetUnit={def:{epDamageResistance:ep}};
 const damages=['phys','arts','true','elemental'].map(type=>({type,mitigate:bits(calc.mitigate(dmg.amount,type,target,{...dmg,elementalRes:ep})),input:bits(calc.attackInput(dmg.amount,type,s)),expected:bits(calc.officialExpectedAmount(dmg,type,target,targetUnit)),base:bits(calc.calculateBaseDamage(dmg,type,target,s,targetUnit)),noSource:bits(calc.calculateBaseDamage(dmg,type,target,null,targetUnit)),estimate:bits(calc.estimateTargetDamage(dmg.amount,type,s))}));
 out({base,buffs:serialized,cm,floor,first,hp,second,hpAfter,target:encoded(target),dmg,ep,damages});
}
