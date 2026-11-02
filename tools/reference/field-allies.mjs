// Concrete native solo field oracle with original default operators and all
// admitted enemy kits. Includes full Battle phases and long skill/redeploy fights.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]),imp=p=>import(pathToFileURL(path.join(root,p)));
const{Battle}=await imp('server/sim/Battle.js'),content=await imp('server/sim/content/enemies.js');
const{kitStealth,kitRefraction}=await imp('server/sim/content/enemies/archetypes.js');
const data=Object.fromEntries(['config','chess','bonds','garrisons','items','bands','effects','choices','enemies','factions','waves','stages','bosses','tokens','backups'].map(k=>[k,JSON.parse(fs.readFileSync(path.join(root,'data',k+'.json'),'utf8'))]));
const keys=Object.keys(data.enemies).filter(k=>content.STATS_ONLY[k]||content.KITS[k]===kitStealth||content.KITS[k]===kitRefraction||k==='enemy_9011_acrefr'||/^enemy_(1196_msfyin|1198_msfshu|1200_msfjin|1202_msfzhi|1204_msfhu|1208_msfji|1210_msfden|1148_dssbr|1305_mhslim|2021_syfish|1229_darmy)(_2)?$/.test(k));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
const encode=x=>typeof x==='number'?(Number.isFinite(x)?bits(x):null):Array.isArray(x)?x.map(encode):x&&typeof x==='object'?Object.fromEntries(Object.entries(x).map(([k,v])=>[k,encode(v)])):x;
const stage={rows:Array(19).fill('r'.repeat(21)),legend:{}};
const cases=[...keys.map((key,i)=>({key,seed:i+21,long:false})),...['enemy_1007_slime',keys.find(k=>k.includes('lurker')),'enemy_1148_dssbr','enemy_1305_mhslim','enemy_2021_syfish','enemy_1229_darmy'].map((key,i)=>({key,seed:i+600,long:true}))];
const ranged=['1_03','1_11','1_14','1_07'].flatMap(k=>['a','b'].map(v=>'chess_char_'+k+'_'+v));
for(const operator of ranged)for(const dir of ['RIGHT','UP','LEFT','DOWN'])cases.push({key:'enemy_1007_slime',operator,dir,seed:1000+cases.length,long:true});
for(const c of cases){
 if(!data.enemies[c.key])throw Error('Missing oracle enemy '+c.key);
 let routes=[{start:[10,10],end:[10,0]},{start:[11,10],end:[11,0]}];
 let spawns=[{enemyKey:c.key,time:0,count:2,interval:.2,mods:{hpMul:c.long?100:1.5,atkMul:c.long?.7:3}},
  {enemyKey:'enemy_1007_slime',time:.3,routeIndex:1,mods:{hpMul:c.long?100:1.5,atkMul:1.5}}];
 let units=[{chessId:'chess_char_1_02_a',uid:1,row:10,col:6},{chessId:'chess_char_1_02_b',uid:2,row:11,col:6}];
 if(c.operator){
  const home=c.dir==='DOWN'?[11,5]:[10,5],vec={RIGHT:[0,1],UP:[1,0],LEFT:[0,-1],DOWN:[-1,0]}[c.dir];
  const distance=c.operator.includes('1_07')&&['UP','LEFT'].includes(c.dir)?1:2;
  const target=[home[0]+distance*vec[0],home[1]+distance*vec[1]];
  units=[{chessId:c.operator,uid:1,row:home[0],col:home[1],dir:c.dir}];
  routes=[{start:target,end:[10,0],checkpoints:[{type:'WAIT',time:150}]}];
  spawns=[{enemyKey:c.key,time:c.dir==='UP'?70:0,count:4,interval:.2,mods:{hpMul:500,atkMul:.1}}];
 }
 const options={stage,routes,spawns,seed:c.seed,timeLimit:c.operator?110:c.long?150:12,players:[{playerId:'p',units}]};
 const b=new Battle({...options,data,content:'full',recordEvents:false});
 const report=()=>{const r=b.result(),p=r.perPlayer.p,pp={};for(const k of['killed','total','leaked','perfect','killedInTotal','leakedInTotal','resolved','creditedKills','coins'])pp[k]=p[k];return {...Object.fromEntries(['time','reason','killed','total','resolved','errors'].map(k=>[k,r[k]])),perPlayer:{p:pp},...(r.unspawned?{unspawned:r.unspawned}:{})};};
 const view=()=>({time:b.time,ticks:b.tickCount,finished:b.finished,reason:b.reason,rng:b.rng.state(),dp:b.players[0].dp,projectiles:b.projectiles.list.length,
  enemies:b.enemies.filter(e=>e.alive).map(e=>{const s=e.s;return{id:e.id,key:e.defId,pos:[e.y,e.x],hp:e.hp,maxHp:s.maxHp,atk:s.atk,res:s.res,cd:e.atkCd,moving:e.moving,hidden:e.hidden,leg:e.route.legIdx,spawn:e.spawnSeq,deploy:e.deploySeq,element:['burn','neural','necrosis','apoptosis','erosion'].map(k=>e.elem[k]),flags:Object.keys(s.flags).filter(k=>s.flags[k]).sort(),buffs:e.buffs.map(x=>({key:x.key,left:x.timeLeft,stacks:x.stacks,persist:!!x.persist}))};}),
  allies:b.allyUnits.map(u=>{const s=u.s,k=u.skill;return{id:u.id,key:u.defId,hp:u.hp,maxHp:s.maxHp,atk:s.atk,def:s.def,res:s.res,cd:u.atkCd,
   alive:u.alive,deployed:u.deployed,removed:u.removed,blocking:u.blocking.map(e=>e.id),deploy:u.deploySeq,aggro:u.aggroSeq,body:u.body??null,
   death:u.deathAt,respawn:u.respawnAt,reason:u.removeReason??null,ground:u.ground,element:['burn','neural','necrosis','apoptosis','erosion'].map(k=>u.elem[k]),
   flags:Object.keys(s.flags).filter(k=>s.flags[k]).sort(),buffs:u.buffs.map(b=>({key:b.key,left:b.timeLeft,stacks:b.stacks,persist:!!b.persist})),
   sp:k.sp,charges:k.charges,left:k.timeLeft,active:k.active,activations:k.activations,attacks:u.stats.attacks,dealt:u.stats.dmg,taken:u.stats.taken};}),report:report()});
 const states=[],ops=[];let killedShooter=false;
 for(let t=0;t<(c.operator?3300:c.long?4500:360)&&!b.finished;t++){
  if((c.long&&!c.operator&&t===1800)||(c.operator&&c.dir==='LEFT'&&t===1201)){b.retreat(b.allyUnits[0]);ops.push({tick:t,op:'retreat',id:b.allyUnits[0].id});}
  if(c.operator&&c.dir==='DOWN'&&!killedShooter&&b.allyUnits[0].skill.active&&b.projectiles.list.some(p=>p.source===b.allyUnits[0])){
   b.dealDamage(null,b.allyUnits[0],{amount:1e9,type:'true',noSp:true});ops.push({tick:t,op:'damage',id:b.allyUnits[0].id,amount:1e9});killedShooter=true;
  }
  b.step();states.push(encode(view()));
 }
 if(b.errorCount)throw Error(c.key+': '+b.errorCount+' oracle errors');
 if(c.operator&&c.dir==='DOWN'&&!killedShooter)throw Error('Missing in-flight source death '+c.operator);
 console.log(JSON.stringify({options,ops,states}));
}
