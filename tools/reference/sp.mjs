// Real SkillRuntime SP methods; no skill content callbacks or matches.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{SkillRuntime}=await imp('server/sim/skills.js');const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<768;i++){
 const d={i,kind:['instant','charges','duration','ammo','toggle','passive'][i%6],cost:i%11,charges:1+i%3,noSkill:i%37===0,noSp:i%7===0,active:i%5===0,hook:i%3===0,mutate:i%13===0,total:i%31+.25};
 const u=new Unit({id:1,side:'ally',kind:'op'});u.deployed=true;u.buffs=[{stacks:1,mods:{spCostFlat:i%3-1},flags:{noSp:d.noSp},shield:0}];let skill;const trace=[];
 const b={_hooks:d.hook?{spGain:true}:{},emit(name,e){trace.push([name,e.reason,bits(e.amount)]);e.amount*=.5;if(d.mutate){skill.spCostMul=.5;}}};
 skill=new SkillRuntime(b,u,{spCost:d.cost,maxCharges:d.charges},d.noSkill?null:{kind:d.kind,spCost:d.cost,charges:d.charges});
 skill.setSpTotal(d.total);skill.active=d.active;
 const states=[];const snap=result=>{states.push({result:result==null?null:bits(result),sp:bits(skill.sp),charges:bits(skill.charges),cost:bits(skill.spCost),total:bits(skill.spTotal),ready:skill.ready,mul:bits(skill.spCostMul)});};snap(null);
 snap(skill.gainSp(1/30,'time'));snap(skill.gainSp(5,'gift',false,{ignoreLock:true}));snap(skill.gainSp(3,'init',true));
 skill.spCostMul=.7;snap(null);skill.setSpTotal(7.5);snap(null);skill.addCharge(.5);snap(null);skill.spCostMul=NaN;snap(null);skill.gainSp(100,'hurt');snap(null);
 process.stdout.write(JSON.stringify({scenario:d,flat:i%3-1,states,trace})+'\n');
}
