// Original default-operator lifecycle oracle. Full content installation; drive
// deployment, skill, buff and HP boundaries directly, without allied AI/rendering.
import fs from 'node:fs';import path from 'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);
const{Battle}=await import(pathToFileURL(path.join(root,'server/sim/Battle.js')));
const data=Object.fromEntries(['config','chess','bonds','garrisons','items','bands','effects','choices','enemies','factions','waves','stages','bosses','tokens','backups'].map(k=>[k,JSON.parse(fs.readFileSync(path.join(root,'data',k+'.json'),'utf8'))]));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(const key of ['chess_char_1_02_a','chess_char_1_02_b'])for(let mode=0;mode<2;mode++){
 const orderRule=mode?'rowBottomUp':'column';
 const inputData=mode?{...data,config:{...data.config,version:{...data.config.version,rules:{deployOrder:orderRule}}}}:data;
 const b=new Battle({seed:42,content:'full',recordEvents:false,autoFinish:false,data:inputData,
  stage:{rows:Array(19).fill('r'.repeat(21)),legend:{}},
  flags:{dpInit:99,dpMax:99,dpPerSec:0},players:[{playerId:'p',units:[{chessId:key,uid:1,row:10,col:5},{chessId:key,uid:2,row:12,col:5},{chessId:key,uid:3,row:9,col:4}]}]});
 b.start();const u=b.allyUnits[0];
 const view=()=>{const s=u.s,k=u.skill;return{hp:bits(u.hp),alive:u.alive,deployed:u.deployed,removed:u.removed,
  stats:Object.fromEntries(['maxHp','atk','def','res','blockCnt','interval'].map(n=>[n,bits(s[n])])),
  order:b.allyUnits.map(a=>[a.uid,a.deploySeq]),dp:bits(b.players[0].dp),deploy:u.deploySeq,aggro:u.aggroSeq,respawn:bits(u.respawnAt),death:bits(u.deathAt),
  reason:u.removeReason??null,body:u.body??null,ground:u.ground,element:Object.values(u.elem).map(bits),
  skill:{active:k.active,sp:bits(k.sp),charges:bits(k.charges),left:bits(k.timeLeft),activations:k.activations??null,readyAt:bits(k.opReadyAt)},
  buffs:u.buffs.map(x=>({key:x.key,mods:Object.fromEntries(Object.entries(x.mods??{}).map(([k,n])=>[k,bits(n)])),persist:!!x.persist}))};};
 const states=[view()],ops=[];
 for(let t=0;t<4500;t++){
  b.time=t/30;
  const damage=(amount,type='true',noSp=false)=>{b.dealDamage(null,u,{amount,type,noSp});ops.push({tick:t,op:'damage',amount,type,noSp});};
  if(mode===0){
   if(t===1500)damage(123.5);
   if(t===1510)damage(0,'arts');
   if(t===2500)damage(1000000);
  }else{
   if(t===10){u.skill.gainSp(100,'talent');ops.push({tick:t,op:'gain',amount:100});}
   if(t===20)damage(0);
   if(t===90)damage(0,'true',true);
   if(t===91)damage(0);
   if(t===120){b.addBuff(u,{key:'discard-on-death',duration:100,mods:{defFlat:21}});ops.push({tick:t,op:'buff'});}
   if(t===180){b.retreat(u);ops.push({tick:t,op:'retreat'});}
   if(t===2100){u.skill.gainSp(100,'talent');ops.push({tick:t,op:'gain',amount:100});damage(1000000);}
  }
  b._tickBuffs(1/30);
  if(u.alive&&u.deployed)u.skill.tick(1/30);
  b._checkRedeploys();states.push(view());
 }
 if(b.errorCount)throw Error('oracle errors '+b.errorCount);
 console.log(JSON.stringify({key,raw:data.chess[key],orderRule,ops,states}));
}
