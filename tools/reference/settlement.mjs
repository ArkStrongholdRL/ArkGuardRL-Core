// Canonical buff/HP pipeline with nested damage and snapshot callbacks.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{BattleStatus}=await imp('server/sim/battle/status.js');const{makeBuff}=await imp('server/sim/buffs.js');const{makeDamageInfo}=await imp('server/sim/attack/helpers.js');const{resolveDamage}=await imp('server/sim/attack/hit.js');const{createRng}=await imp('server/sim/rng.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<256;i++){
 const base=makeBuff({}).seq,u=new Unit({id:1,side:'ally',kind:'op',base:{maxHp:10000,def:20,res:30}});u.deployed=true;const trace=[];let nesting=false;
 const b={units:[u],time:0,kind:'normal',rng:createRng(i+1),_hooks:{block:true,dodge:true},_safe:f=>f(),_internalError(k,e){throw e;},_pp:()=>null,_skills:{onDamaged(){}},_ev(e){if(e[0]==='status')trace.push(e);},fx(){},kill(t){t.hp=0;t.alive=false;t.deployed=false;trace.push(['kill']);},emit(name,c){trace.push([name,c.cancelled]);},dealDamage(d){return resolveDamage(this,{source:null,credit:null,target:u,dmg:makeDamageInfo(d)});}};
 for(const key of['addBuff','removeBuff','_removeBuffAt','_tickBuffs'])b[key]=BattleStatus.prototype[key];
 function spec(d){return {...d,duration:d.duration==='inf'?Infinity:d.duration,injury:d.injury?e=>{
  trace.push(['injury',e.buff.seq-base,bits(e.amount)]);
  if(d.injury===1)e.amount*=.7;
  if(d.injury===2)e.block(.2);
  if(d.injury===3){e.defer(()=>trace.push(['cleanup',7]));e.amount=Math.max(0,e.amount-3);}
  if(d.injury===4&&!nesting){b.removeBuff(u,e.buff);nesting=true;try{b.dealDamage({amount:4,type:'true',noSp:true});}finally{nesting=false;}}
  if(d.injury===5)b.removeBuff(u,'b');
  if(d.injury===6&&!u.findBuff('late'))b.addBuff(u,spec({key:'late',duration:.3,mods:{dmgTakenMul:.8},observe:true}));
 }:null,observeInjury:d.observe?e=>{trace.push(['observe',e.buff.seq-base,bits(e.amount),e.cancelled,e.outcome,u.buffs.includes(e.buff)]);e.buff.shield+=2;u.markDirty();}:null,onRemove:d.remove?({buff})=>trace.push(['remove',buff.seq-base,bits(buff.timeLeft)]):null};}
 const descriptors=[
  {key:'shield',duration:'inf',shield:25,shieldHits:i%3,shieldType:i%4===0?['phys','arts']:null,visible:true,mods:i%5===0?{}:null,flags:i%7===0?{}:null,onRemove:null,remove:true,keepEmptyShield:i%11===0},
  {key:'b',duration:1.1,stacks:2,mods:{dmgTakenMul:.8,dodgePhys:.25,dodgeArts:.25},damageReduction:{phys:5,arts:10,true:3,elemental:2},reductionGroup:'same',injuryPriority:i%2?2:0,observe:i%3===0},
  {key:'c',duration:1.5,mods:null,damageReduction:{phys:8,arts:7,true:4,elemental:6},reductionGroup:'same',injuryPriority:1,injury:1+i%6,observe:true,remove:true}
 ];for(const d of descriptors)b.addBuff(u,spec(d));
 const ops=[];for(let j=0;j<18;j++){if(j%3===2)ops.push({tick:.2});else if(j%5===4)ops.push({add:{key:'shield',duration:.5,shield:35,visible:true,refresh:['replace','extend','stack'][i%3],maxStacks:3,remove:true}});else ops.push({hit:{amount:40+j*3,type:['phys','arts','true','elemental'][(i+j)%4],dodgeMode:['normal','ignore','uncancellable'][(i+j)%3],ignoreCancelReasons:i%13===0?['block']:[],noSp:true}});}
 const states=[];for(const op of ops){let result=null;if(op.tick!=null){b.time+=op.tick;b._tickBuffs(op.tick);}else if(op.add)b.addBuff(u,spec(op.add));else result=b.dealDamage(op.hit);const s=u.s;states.push({result:result==null?null:bits(result),hp:bits(u.hp),taken:bits(u.stats.taken),rng:b.rng.state(),shield:bits(s.shield),buffs:u.buffs.map(x=>({id:x.seq-base,key:x.key,stacks:x.stacks,shield:bits(x.shield),hits:x.shieldHits,left:bits(x.timeLeft)}))});}
 process.stdout.write(JSON.stringify({i,descriptors,ops,states,trace})+'\n');
}
