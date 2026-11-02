// Real strongest-effect waiting chains, expiry resumption and resist decay.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{BattleStatus}=await imp('server/sim/battle/status.js');const{makeBuff,STATUS}=await imp('server/sim/buffs.js');const{Unit}=await imp('server/sim/units.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
const names=['slow','fragile','artsFragile','physFragile','elemFragile','weaken','aspdDown','defDown','resDown','resist','plain'];
for(let i=0;i<384;i++){
 const name=names[i%names.length],plain=name==='plain',base=makeBuff({}).seq,u=new Unit({id:1,side:'ally',base:{maxHp:1000,atk:100,def:100,res:30,moveSpeed:1}});u.deployed=true;const trace=[];
 const b={units:[u],time:0,_safe:f=>f(),_ev(e){trace.push(e);}};for(const key of['addBuff','removeBuff','_removeBuffAt','_tickBuffs','_applyValuedStatus','applyStrongest'])b[key]=BattleStatus.prototype[key];
 if(name==='resist')b.addBuff(u,{key:'palsy',stacks:3,maxStacks:3,status:'palsy',visible:true});
 const ops=[];for(let j=0;j<48;j++){
  if(j%4===2)ops.push({op:'tick',dt:[.1,.3,1,5][(i+j)%4]});
  else if(j%17===0)ops.push({op:'raw',duration:.2});
  else if(j%11===0)ops.push({op:'remove'});
  else ops.push({op:'apply',duration:[.2,.5,2,6,'inf'][(i*3+j)%5],value:[.1,.3,-.3,.7,1.1,-30,20][(i+j*3)%7],stackAs:(i+j)%5===0?.9:null,source:j%2?1:null});
 }
 const states=[];const duration=x=>x==='inf'?Infinity:x;
 for(const op of ops){
  if(op.op==='tick'){b.time+=op.dt;b._tickBuffs(op.dt);}else if(op.op==='remove')b.removeBuff(u,name);else if(op.op==='raw')b.addBuff(u,{key:name,status:plain?null:name,duration:op.duration,mods:{atkPct:.01}});
  else if(plain)b.applyStrongest(u,name,{duration:duration(op.duration),value:op.value,mods:v=>({atkPct:v}),source:op.source?u:null});
  else b._applyValuedStatus(u,name,STATUS[name],duration(op.duration),op.value,op.source?u:null,op.stackAs);
  const s=u.s;states.push({stats:['atk','def','res','moveSpeed','dmgTakenMul','artsTakenMul','physTakenMul','elementalTakenMul','aspd'].map(k=>bits(s[k])),buffs:u.buffs.map(x=>{const tail=[];for(let t=x.data.tail;t;t=t.tail)tail.push({value:bits(t.value),as:t.stackAs==null?null:bits(t.stackAs),until:bits(t.until)});return{id:x.seq-base,key:x.key,stacks:x.stacks,duration:bits(x.duration),left:bits(x.timeLeft),acc:bits(x._acc),value:x.data.value==null?null:bits(x.data.value),as:x.data.stackAs==null?null:bits(x.data.stackAs),tail};})});
 }
 process.stdout.write(JSON.stringify({i,name,ops,states,trace})+'\n');
}
