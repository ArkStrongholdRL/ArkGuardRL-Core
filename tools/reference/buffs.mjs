// Real BattleStatus add/remove/tick with nested callbacks, no assets or match.
import path from 'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{BattleStatus}=await imp('server/sim/battle/status.js');const{makeBuff}=await imp('server/sim/buffs.js');const{Unit}=await imp('server/sim/units.js');const{heal}=await imp('server/sim/attack/heal.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<256;i++){
 const base=makeBuff({}).seq;const u=new Unit({id:1,side:'ally',base:{maxHp:1000,hpRegen:i%2?2.3:10,atk:60}});u.hp=950;u.deployed=true;const trace=[];let nested=false;
 const b={units:[u],time:0,_hooks:{},_safe:fn=>fn(),_ev(e){if(e[0]==='status')trace.push(e);},_pp:()=>null,heal(source,target,amount,opts){return heal(this,source,target,amount,opts);}};
 for(const key of ['addBuff','removeBuff','_removeBuffAt','_tickBuffs'])b[key]=BattleStatus.prototype[key];
 function callback(token,event,{unit,buff,dt}){
  trace.push([event,buff.seq-base,bits(dt??0),bits(buff.timeLeft),bits(buff._acc),unit.buffs.includes(buff)]);
  if(event!=='tick'){
   if(token===4)b.removeBuff(unit,'b');
   if(token===5)b.addBuff(unit,desc({key:buff.key,duration:.4,tick:1,mods:{atkPct:.2}}));
   return;
  }
  if(token===2)b.removeBuff(unit,buff);
  if(token===3&&!buff.data.spawned){buff.data.spawned=true;b.addBuff(unit,desc({key:'child',duration:.15,tick:1,mods:{atkPct:.1}}));}
  if(token===6)b.addBuff(unit,desc({key:buff.key,duration:.2,tick:1,mods:{atkPct:.3}}));
  if(token===7){unit.hp=0;unit.alive=false;unit.deployed=false;}
  if(token===8&&!nested){nested=true;try{b._tickBuffs(dt/2);}finally{nested=false;}}
 }
 function desc(d){return{key:d.key,duration:d.duration==='inf'?Infinity:d.duration,stacks:d.stacks,maxStacks:d.cap,refresh:d.refresh,mods:d.mods??null,flags:d.flags??null,shield:d.shield,shieldHits:d.hits,persist:d.persist,visible:d.visible,status:d.status,allowDead:d.allowDead,interval:d.interval,onTick:d.tick?c=>callback(d.tick,'tick',c):null,onRemove:d.remove?c=>callback(d.remove,'remove',c):null,onExpire:d.expire?c=>callback(d.expire,'expire',c):null};}
 const ops=[];for(let j=0;j<36;j++){
  if(j%5<2){const tick=[null,1,2,3,6,8,i%16===0?7:1][(i+j)%7];ops.push({op:'add',key:['a','b','a','c'][(i+j)%4],duration:[.05,.2,.5,'inf',0,-1][(i+j)%6],stacks:1+(i+j)%3,cap:1+(i+j)%4,refresh:['replace','extend','stack','independent','keep'][(i+Math.floor(j/5))%5],mods:j%3===0?null:j%3===1?{}:{atkPct:.1},flags:j%3===0?null:j%3===1?{}:{noHeal:true},shield:(i+j)%3*10,hits:(i+j)%2,persist:i%3===0,visible:true,status:i%11===0?'shared':null,allowDead:j%3===0,interval:[0,.1,.01][(i+j)%3],tick,remove:(i+j)%3===0?4:1,expire:(i+j)%3===0?5:1});}
  else if(j%5===3)ops.push({op:'remove',key:['a','b','c'][j%3]});else ops.push({op:'tick',dt:[0,.03,.4,.5][(i+j)%4]});
 }
 const states=[];for(const op of ops){if(op.op==='add')b.addBuff(u,desc(op));else if(op.op==='remove')b.removeBuff(u,op.key);else{b.time+=op.dt;b._tickBuffs(op.dt);}const s=u.s;states.push({hp:bits(u.hp),regen:bits(u._regenAcc??0),alive:u.alive,atk:bits(s.atk),buffs:u.buffs.map(x=>({id:x.seq-base,key:x.key,stacks:x.stacks,cap:x.maxStacks,duration:bits(x.duration),left:bits(x.timeLeft),acc:bits(x._acc),shield:bits(x.shield),hits:x.shieldHits}))});}
 process.stdout.write(JSON.stringify({i,ops,states,trace})+'\n');
}
