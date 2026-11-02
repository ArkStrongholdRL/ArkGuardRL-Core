// Development-only normalization/spawn oracle; no content abilities are exercised.
import fs from 'node:fs';import path from 'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);
const{normalizeEnemy}=await import(pathToFileURL(path.join(root,'server/sim/simdata.js')));
const{Battle}=await import(pathToFileURL(path.join(root,'server/sim/Battle.js')));
const raw=JSON.parse(fs.readFileSync(path.join(root,'data/enemies.json'),'utf8'));
Object.assign(raw,{
 enemy_alias:{rank:'boss',stats:{hp:'1234',attackSpeed:0,baseAttackTime:'1.3',range:'2.2',spd:'0.8',magicResistance:'23',elementRes:'11',elementDmgRes:12,immunities:{stunImmune:true},otherImmunities:['sleepImmune']},dmg:'ELEMENT',hitArea:{w:'2',h:3,dx:0.2,dy:-0.1},attackAnim:{dur:1,hit:3},attackMoves:true},
 enemy_heal:{applyWay:'RANGED',dmg:'HEAL',hp:345,atk:100,range:4,motion:'FLY',immune:['freezeImmune'],extra:{epResistance:12,epDamageResistance:23,hpRecoveryPerSec:7}},
});
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
let i=0;
for(const[key,r]of Object.entries(raw)){
 const overrides=i%3?{}:{stats:{hp:1234.5,atk:234.6,def:34.7,res:80,aspd:123,bat:0.7,rangeRadius:2.3,moveSpeed:0.65,massLevel:4,blockCnt:3,lpr:2}};
 const mods=i%2?{hpMul:1.25,atkMul:0.8,defMul:2,resMul:0,speedMul:1.3}:{hpMul:0,atkMul:-2,defMul:-1};
 const b=new Battle({content:'none',recordEvents:false,data:{enemies:{[key]:r}},enemyOverrides:{[key]:overrides},logger:{warn(){},error(){}}});
 const e=b.spawnEnemy(key,{pos:[10,6],mods}),d=e.def;
 const base={maxHp:d.maxHp,atk:d.atk,def:d.def,res:d.res,aspd:d.aspd,bat:d.bat,rangeRadius:d.rangeRadius,moveSpeed:d.moveSpeed,massLevel:d.massLevel,blockCnt:0,spRecovery:0,tauntLevel:d.tauntLevel,hpRecoveryPerSec:d.hpRecoveryPerSec};
 const output={key,base,dmgType:d.dmgType,melee:d.applyWay==='MELEE',motion:d.motion,leader:d.rank==='BOSS',lpr:d.lpr,blockWeight:d.blockCnt,immune:[...d.immune].sort(),epResistance:d.epResistance,elementalResistance:d.epDamageResistance,notCountInTotal:d.notCountInTotal,hitArea:d.hitArea,attackAnim:d.attackAnim,attackMoves:d.attackMoves};
 const unit={base:Object.fromEntries(Object.keys(base).map(k=>[k,bits(e.base[k])])),hp:bits(e.hp),blockWeight:bits(e.blockWeight),fly:e.isFlying,immune:[...d.immune].sort(),position:[bits(e.y),bits(e.x)]};
 console.log(JSON.stringify({key,raw:r,overrides,mods,output,unit}));i++;
}
