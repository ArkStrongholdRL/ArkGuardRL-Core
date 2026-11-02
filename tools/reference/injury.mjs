// Ordered injury oracle: no matches, assets or model loading.
import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{injuryEffects,settleEffects}=await imp('server/sim/attack/effects.js');const{createRng}=await imp('server/sim/rng.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};const random=createRng(843);
for(let i=0;i<1024;i++){
 const type=['phys','arts','true','elemental'][i%4],mode=['ignore','normal','uncancellable'][Math.floor(i/4)%3],ignoreBlock=!!(i%2);
 const descriptors=Array.from({length:1+i%8},(_,j)=>({id:j+1,key:'group'+j%2,priority:j%3,stacks:1+j%3,
  mods:j%3===0?null:{dmgTakenMul:.5+random(),physTakenMul:.5+random(),artsTakenMul:.5+random(),elementalTakenMul:.5+random(),dodgePhys:random(),dodgeArts:random()},
  shield:random()<.5?random()*80:0,hits:random()<.2?1:0,shieldTypes:j%3===0?null:['phys','arts'],keep:!!(j%2),
  reduction:j%4===0?random()*20:null,action:j%5,observer:!!(j%2),scale:random()*2,probability:random(),remove:((j+1)%(1+i%8))+1}));
 const trace=[];const rng=createRng(i+1);const target={buffs:[],markDirty(){trace.push(['dirty']);}};
 const remove=id=>{target.buffs=target.buffs.filter(b=>b.id!==id);};
 const buffs=descriptors.map(d=>({id:d.id,key:d.key,injuryPriority:d.priority,seq:d.id,stacks:d.stacks,mods:d.mods,shield:d.shield,shieldHits:d.hits,shieldType:d.shieldTypes,keepEmptyShield:d.keep,
 damageReduction:d.reduction==null?null:{[type]:d.reduction},
 injury:d.action===0?null:e=>{trace.push(['injury',d.id,bits(e.amount)]);if(d.action===1)e.amount*=d.scale;if(d.action===2)e.block(d.probability);if(d.action===3)e.dodge(d.probability);if(d.action===4)remove(d.remove);e.defer(()=>trace.push(['cleanup',d.id]));},
 observeInjury:d.observer?e=>trace.push(['observe',d.id,bits(e.amount),e.cancelled,e.outcome]):null}));
 target.buffs=buffs.slice();
 const battle={rng,_safe(fn){fn();},_internalError(label,e){throw e;},_removeBuffAt(t,index){t.buffs.splice(index,1);trace.push(['remove',buffs.find(b=>!t.buffs.includes(b)&&!b._removed)?.id??0]);for(const b of buffs)if(!t.buffs.includes(b))b._removed=true;}};
 const amount=i%29===0?0:random()*200;
 const result=injuryEffects(battle,target,amount,type,{dodgeMode:mode,context:{dmg:{ignoreCancelReasons:ignoreBlock?['block']:[]}},onBlock:c=>trace.push(['block',c]),onDodge:(c,a)=>trace.push(['dodge',c,bits(a)])});
 // Dirty is a cache invalidation detail, tracked separately from observable callbacks.
 const dirty=trace.some(x=>x[0]==='dirty');const settled=settleEffects(battle,result,a=>{trace.push(['apply',bits(a)]);return a;});
 const state=buffs.map(b=>({id:b.id,active:target.buffs.includes(b),shield:bits(b.shield),hits:b.shieldHits}));
 process.stdout.write(JSON.stringify({i,type,mode,ignoreBlock,descriptors,amount,result:{amount:bits(result.amount),cancelled:result.cancelled,outcome:result.outcome,notifyZero:result.notifyZero},settled:bits(settled),state,dirty,rng:rng.state(),trace:trace.filter(x=>!['dirty','remove'].includes(x[0]))})+'\n');
}
