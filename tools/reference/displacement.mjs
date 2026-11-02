// Headless force/weight/collision oracle. No fx or display state is recorded.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);const{Battle}=await import(pathToFileURL(path.join(root,'server/sim/Battle.js')));
const{Unit}=await import(pathToFileURL(path.join(root,'server/sim/units.js')));
const{pushTiles,pushUnbalance,pullUnbalance}=await import(pathToFileURL(path.join(root,'server/sim/battle/displacement.js')));
const stages=JSON.parse(fs.readFileSync(path.join(root,'data/stages.json'),'utf8'));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<320;i++){
 const stageId=Object.keys(stages)[i%13],position=i%3?[10.05,5.1]:[10,8];
 const cfg={stageId,position,mass:(i%11)*.5,static:i%7===0,boss:i%13===0,ally:i%17===0,dead:i%19===0,fly:i%5===0,noDisplace:i%23===0,hold:i%5===0?2:null,dir:['RIGHT','UP','LEFT','DOWN'][i%4]};
 const b=new Battle({stage:{...stages[stageId],devices:[]},content:'none',recordEvents:false});
 let ended=0;b._stealthSwitch=()=>{ended++;};b.fx=()=>{};
 const e=new Unit({id:1,side:cfg.ally?'ally':'enemy',kind:'enemy',x:position[1],y:position[0],motion:cfg.fly?'FLY':'WALK',def:{staticBody:cfg.static},base:{maxHp:1000,massLevel:cfg.mass}});
 e.alive=!cfg.dead;e.deployed=true;e.isBoss=cfg.boss;e.unbalanceUntil=cfg.hold??-Infinity;e.atkStandUntil=3;
 e.route={pts:[[10,5]],legs:[],legIdx:0};if(cfg.noDisplace)e.buffs=[{key:'immune',flags:{noDisplace:true}}];
 const a=new Unit({id:2,side:'ally',kind:'op',x:5,y:10,dir:cfg.dir,base:{blockCnt:3,maxHp:1000}});a.deployed=true;a.ground=true;
 b.units=[e,a];b.allyUnits=[a];b.enemies=[e];b._occ[10*21+5]=a;b._checkBlock(e);
 const ops=[],states=[];
 for(let j=0;j<20;j++){
  b.time=j*.07;e.atkStandUntil=b.time+.4;
  const force=((i+j)%15-5)*.5,dir={x:[1,-1,0,.7][j%4],y:[0,.7,-1,1][(i+j)%4]};
  const from=j%5===0?{x:e.x,y:e.y,fwd:a.fwd}:a;
  const op={op:j%7,force,dir,from:{position:[from.y,from.x],forward:from.fwd??null},fixed:i%3===0,fixedAngle:i%4===0,inward:i%6===0,effect:i%2===0,to:[9+(i%4),2+(j%9)],stop:[-.1,0,.6708,2][j%4],distance:((i+j)%11-2)*.43};
  let moved;
  switch(op.op){
   case 0:moved=b.push(e,force,{from,inward:op.inward,effect:op.effect});break;
   case 1:moved=b.push(e,force,{from,dir,fixed:op.fixed,fixedAngle:op.fixedAngle,effect:op.effect});break;
   case 2:moved=b.pull(e,force,{to:{x:op.to[1],y:op.to[0]},center:a,stop:op.stop});break;
   case 3:moved=b.pullToFront(e,a,force);break;
   case 4:moved=b.displace(e,dir,op.distance);break;
   case 5:moved=b.pushDistance(e,force,{effect:op.effect});break;
   case 6:b._unbalance(e,op.distance);moved=0;break;
  }
  ops.push(op);states.push({moved:bits(moved),position:[bits(e.y),bits(e.x)],until:bits(e.unbalanceUntil),stand:bits(e.atkStandUntil),blocked:e.blockedBy?.id??null,blocking:a.blocking.map(e=>e.id),cached:!!e.route.pts,ended,level:bits(b.forceLevel(e,force)),tables:[pushTiles(force,op.effect),pushUnbalance(force),pullUnbalance(force)].map(bits)});
 }
 console.log(JSON.stringify({cfg,ops,states}));
}
