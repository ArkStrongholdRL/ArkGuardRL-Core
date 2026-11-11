// Read-only geometry projection and explicit offline teacher takeover.
export const ROUTE_SCHEMA='arkguardrl/own-field-route-geometry/v2';
export function geometry(m,ps,bot,revision){
 const routes=m.round>0?bot.fieldModel(m,ps).routes:[];
 const unique=new Map();
 // The bot's weighted fieldModel omits routes without a base spawn, including
 // flight paths later used by visible bounties. Include current-wave FLY geometry
 // independently of spawn counts. No future waves or enemy properties are exposed.
 const group=m.wave?null:m.bossGroupOf(ps);const wave=m.wave||group?.wave;
 const field=m.deployFieldOf(ps);const right=field==='bossR';
 const map=([r,c])=>field==='normal'?[r,c]:[r+7,right?20-c:c];
 function trace(points){const tiles=[];for(let i=0;i+1<points.length;i++){
  const a=points[i],b=points[i+1],n=Math.max(1,Math.abs(a[0]-b[0]),Math.abs(a[1]-b[1]));
  for(let j=0;j<=n;j++){const x=map([Math.round(a[0]+(b[0]-a[0])*j/n),Math.round(a[1]+(b[1]-a[1])*j/n)]);
   if(x[0]>=9&&x[0]<=12&&x[1]>=2&&x[1]<=10&&JSON.stringify(x)!==JSON.stringify(tiles.at(-1)))tiles.push(x);
  }
 }return tiles}
 for(const rt of wave?.routes||[]){
  if(rt.motion!=='FLY'||!rt.start||!rt.end)continue;
  // Boss pair routes belonging to the other protection point are not own routes.
  if(field!=='normal'&&((right&&rt.end[1]<=10)||(!right&&rt.end[1]>10)))continue;
  const cps=(rt.checkpoints||[]).filter(cp=>Array.isArray(cp)||cp.type==='MOVE').map(cp=>Array.isArray(cp)?cp:cp.pos);
  const tiles=trace([rt.start,...cps,rt.end]);const objective=map(rt.end);
  if(JSON.stringify(tiles.at(-1))===JSON.stringify(objective))tiles.pop();
  if(tiles.length){const row={fly:true,tiles};unique.set(JSON.stringify(row),row)}
 }
 for(const r of routes){const tiles=r.tiles.map(k=>k.split(',').map(Number));const row={fly:!!r.fly,tiles};unique.set(JSON.stringify(row),row)}
 const out={schema:ROUTE_SCHEMA,stageId:m.stageId,round:m.round,field:m.deployFieldOf(ps),revision,routes:[...unique.values()]};
 if(out.routes.length>64||out.routes.some(r=>r.tiles.length>256))throw Error('Route geometry budget exceeded');
 return out;
}
export function captureTeacher(m,ps,observe,geo,write){
 let depth=0,rehearsals=0;const restores=[];
 function wrap(obj,name,action,voidOK=false){const original=obj[name];restores.push(()=>obj[name]=original);obj[name]=function(...args){
  if(depth)return original.apply(this,args);const a=action(...args);if(!a)return original.apply(this,args);
  const before=observe(),routeGeometry=geo();depth++;let r;
  try{r=original.apply(this,args)}finally{depth--}
  const after=observe(),result=voidOK?{ok:true}:r;
  if(!result||typeof result.ok!=='boolean')throw Error(`Unrecognized teacher reply ${name}`);
  write({kind:'action',method:name,action:JSON.parse(JSON.stringify(a)),before,after,routeGeometry,accepted:result.ok,result});return r;
 }}
 for(const [name,fn] of Object.entries({buy:slot=>({t:'g.buy',slot}),refresh:()=>({t:'g.refresh'}),freeze:()=>({t:'g.freeze'}),levelUp:()=>({t:'g.levelUp'}),sell:uid=>({t:'g.sell',uid}),move:(uid,to,dir)=>({t:'g.move',uid,to,...(to.area==='board'?{dir:dir??to.dir??'RIGHT'}:{})}),equip:(itemUid,targetUid,replaceUid)=>({t:'g.equip',itemUid,targetUid,...(replaceUid?{replaceUid}:{})}),useArt:(itemUid,row,col,dir)=>({t:'g.art',itemUid,row,col,...(dir?{dir}:{})}),destroy:uid=>({t:'g.destroy',uid}),unequip:itemUid=>({t:'g.unequip',itemUid}),pickReward:idx=>({t:'g.reward',idx}),setReady:ready=>({t:'g.ready',ready})}))wrap(ps,name,fn);
 wrap(m,'_applyBand',(p,bandId)=>p===ps?{t:'g.band',bandId}:null,true);
 wrap(m,'_applyCard',(p,idx)=>p===ps?{t:'g.choice',idx}:null,true);
 wrap(m,'pickPersonalChoice',(p,idx,choiceId)=>p===ps?{t:'g.choice',idx,choiceId}:null);
 const original=m.newBattle;restores.push(()=>m.newBattle=original);m.newBattle=function(o,...args){if(o.fieldId?.startsWith('r:'))rehearsals++;return original.call(this,o,...args)};
 return {get rehearsals(){return rehearsals},restore(){for(const f of restores.reverse())f()}};
}
