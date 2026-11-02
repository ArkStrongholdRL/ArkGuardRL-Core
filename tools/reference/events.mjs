// Real battle/hooks.js bus and timers, including reentrant once and owner removal.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const{BattleHooks}=await import(pathToFileURL(path.join(root,'server/sim/battle/hooks.js')));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};const num=x=>x==='inf'?Infinity:x==='nan'?NaN:x;
for(let i=0;i<256;i++){
 const trace=[],handles=[],timers=[];let nesting=false;const b={_hooks:Object.create(null),_sched:[],time:0,_emitDepth:0,_runDepth:0,_frameCtx:[],_frameName:[],_frameOwner:[],_handlerError(label,owner,e){throw e;}};
 for(const key of['on','off','offOwner','hasHook','emit','after','every','_runScheduled','_compactDue','_safe','_chain'])b[key]=BattleHooks.prototype[key];
 const hb=b.on('unused',()=>{}).seq;b.off('unused');const tb=b.after(0,()=>{}).seq;b._sched=[];
 const hookFns=new Map(),timerFns=new Map();
 const hook=token=>{if(!hookFns.has(token))hookFns.set(token,c=>{
  trace.push(['hook',token,c.value,c.depth]);
  if(token===1)c.value++;
  if(token===2&&handles.length)b.off(handles[handles.length-1]);
  if(token===3&&!c.added){c.added=true;handles.push(b.on('B',hook(1),{owner:2}));}
  if(token===4&&c.depth<2){const nested={value:c.value,depth:c.depth+1};b.emit('B',nested);c.value=nested.value;}
  if(token===5)b.offOwner(1);
  if(token===6)c.stopPropagation=true;
  if(token===7&&c.depth<1){const nested={value:c.value,depth:c.depth+1};b.emit('A',nested);c.value=nested.value;}
  if(token===8&&handles.length)b.off(handles[0]);
  if(token===9&&timers.length)timers[timers.length-1].cancel();
 });return hookFns.get(token);};
 const timer=token=>{if(!timerFns.has(token))timerFns.set(token,(b,s)=>{
  trace.push(['timer',token,s.seq-tb,s.count,bits(s.due),s.cancelled]);
  if(token===2&&!nesting){timers.push(b.after(0,timer(1),{owner:2}));nesting=true;try{b._runScheduled();}finally{nesting=false;}}
  if(token===3)s.cancel();if(token===4)b.offOwner(1);if(token===5)b.emit('A',{value:7,depth:0});
  if(token===6){s.due-=.05;s.interval=.1;}if(token===7)s.interval=0;
 });return timerFns.get(token);};
 const ops=[];for(let j=0;j<48;j++){
  const n=(i+j)%8;
  if(n<3)ops.push({op:'on',key:j%2?'A':'B',token:1+(i+j)%9,priority:[0,2,-1,'inf'][j%4],owner:j%3===0?null:j%3,once:j%5===0});
  else if(n===3)ops.push({op:'emit',key:j%2?'A':'B',value:j,depth:0});
  else if(n===4)ops.push({op:'timer',every:j%2===0,seconds:[0,.03,.1,'inf','nan'][(i+j)%5],token:1+j%7,owner:j%3===0?null:j%3,immediate:j%3===0,holds:j%2===0});
  else if(n===5)ops.push({op:'run',dt:[0,.1,.5,2][j%4]});
  else if(n===6)ops.push({op:'off',key:j%2?'A':'B',token:j%3===0?null:1+(i+j)%9});else ops.push({op:'owner',owner:j%3===0?null:j%3});
 }
 // Ensure every seed exercises interval catch-up and an immediate one-shot.
 ops.unshift({op:'timer',every:true,seconds:.03,token:1+i%7,owner:1,immediate:true,holds:false});
 ops.push({op:'run',dt:2},{op:'emit',key:'A',value:1,depth:0},{op:'run',dt:.1});
 const states=[];for(const op of ops){let value=null;if(op.op==='on')handles.push(b.on(op.key,hook(op.token),{priority:num(op.priority),owner:op.owner,once:op.once}));else if(op.op==='emit'){const c={value:op.value,depth:op.depth};b.emit(op.key,c);value=[c.value,!!c.stopPropagation];}else if(op.op==='timer'){const opts={owner:op.owner,immediate:op.immediate,holdsBattle:op.holds};timers.push(op.every?b.every(num(op.seconds),timer(op.token),opts):b.after(num(op.seconds),timer(op.token),opts));}else if(op.op==='run'){b.time+=op.dt;b._runScheduled();}else if(op.op==='off')b.off(op.key,op.token==null?undefined:hook(op.token));else b.offOwner(op.owner);
 states.push({value,hooks:['A','B'].map(k=>(b._hooks[k]??[]).map(h=>h.seq-hb)),timers:b._sched.filter(s=>!s.cancelled).map(s=>({id:s.seq-tb,due:bits(s.due),interval:bits(s.interval),count:s.count??null,holds:s.holdsBattle})),holds:b._sched.some(s=>!s.cancelled&&s.holdsBattle)});
 }
 process.stdout.write(JSON.stringify({i,ops,states,trace})+'\n');
}
