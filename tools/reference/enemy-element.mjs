// Original on-hit elemental kits + real nested HP/gauge/burst settlement.
import fs from'node:fs';import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]),imp=p=>import(pathToFileURL(path.join(root,p)));
const{Battle}=await imp('server/sim/Battle.js'),{Unit}=await imp('server/sim/units.js'),{install}=await imp('server/sim/content/enemies.js');
const enemies=JSON.parse(fs.readFileSync(path.join(root,'data/enemies.json'),'utf8'));
const keys=['enemy_1148_dssbr','enemy_1148_dssbr_2','enemy_1305_mhslim','enemy_1305_mhslim_2','enemy_2021_syfish','enemy_1229_darmy','enemy_1229_darmy_2'];
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(const key of keys)for(let variant=0;variant<6;variant++){
 const overrides=variant%2?{talents:{bb:{'epdamage.attack@ep_damage_ratio':.8,'EpDamage.attack@ep_damage_ratio':.6}}}:{};
 const b=new Battle({seed:42,content:'none',recordEvents:false,data:{enemies},enemyOverrides:{[key]:overrides}});install(b);
 const e=b.spawnEnemy(key,{pos:[10,5]}),a=new Unit({id:2,side:'ally',kind:'op',base:{maxHp:100000,atk:0,def:70,res:20}});a.deployed=true;a.x=5;a.y=10;b.units.push(a);b.allyUnits.push(a);
 const trace=[];
 // Zero is upstream's sentinel for 'no attack identity'; native uses None.
 b.on('damaged',c=>trace.push([c.source?.id??null,c.target.id,!!c.dmg?.isAttack,c.dmg?.attackId||null,c.dmg?.type??'true',bits(c.amount),a.alive,bits(a.hp)]),{priority:100});
 const states=[],ops=[];
 for(let j=0;j<180;j++){
  b.time=j*.1;const op={time:b.time,attack:j%5!==0,amount:j%11===0?0:17.5,kind:['phys','arts','true','elemental'][j%4],attackId:j+1,multi:1+j%3,sourceDead:j>=160,deathTarget:variant===5&&j===140};
  if(j===30){op.atk=700.3;e.base.atk=op.atk;e.markDirty();}
  if(j===50){op.silence=true;b.applyStatus(e,'silence',{duration:1});}
  if(j===85){op.guard=true;b.addBuff(a,{key:'guard',duration:.8,flags:{invulnerable:true}});}
  if(variant===2&&j===65){op.dodge=true;b.addBuff(a,{key:'dodge',duration:3,mods:{dodgePhys:.3,dodgeArts:.4}});}
  if(op.deathTarget)a.hp=1;if(op.sourceDead){e.alive=false;e.deployed=false;}
  for(let k=0;k<op.multi;k++)b.dealDamage(e,a,{amount:op.amount,type:op.kind,isAttack:op.attack,attackId:op.attackId,noSp:true,isProjectile:true});
  b._tickBuffs(.1);void a.s;
  states.push({hp:bits(a.hp),alive:a.alive,gauges:['burn','neural','necrosis','apoptosis','erosion'].map(k=>bits(a.elem[k])),flags:Object.keys(a.s.flags).filter(k=>a.s.flags[k]).sort(),res:bits(a.s.res),def:bits(a.s.def),rng:b.rng.state(),sourceDamage:bits(e.stats.dmg),taken:bits(a.stats.taken),buffs:a.buffs.map(b=>({key:b.key,left:bits(b.timeLeft)}))});ops.push(op);
 }
 console.log(JSON.stringify({key,raw:enemies[key],overrides,ops,states,trace}));
}
