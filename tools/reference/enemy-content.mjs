// Original enemy handlers, HP settlement and block timers; no renderer or match.
import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
const root=path.resolve(process.argv[2]),imp=p=>import(pathToFileURL(path.join(root,p)));
const {Battle}=await imp('server/sim/Battle.js'),{Unit}=await imp('server/sim/units.js');
const content=await imp('server/sim/content/enemies.js');
const {kitStealth,kitRefraction}=await imp('server/sim/content/enemies/archetypes.js');
const {enemyStealthed,canTargetEnemy}=await imp('server/sim/targeting.js');
const data=JSON.parse(fs.readFileSync(path.join(root,'data/enemies.json'),'utf8'));
const selected=Object.keys(data).filter(k=>content.STATS_ONLY[k]||content.KITS[k]===kitStealth||content.KITS[k]===kitRefraction||k==='enemy_9011_acrefr'||/^enemy_(1196_msfyin|1198_msfshu|1200_msfjin|1202_msfzhi|1204_msfhu|1208_msfji|1210_msfden)(_2)?$/.test(k));
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(const key of selected)for(let variant=0;variant<2;variant++){
 const overrides=variant?{talents:{bb:{'refracting.magic_resistance':17.5,'Refracting.magic_resistance':23.5,'Refracting.max_hp':.5}}}:{};
 const mods=variant?{hpMul:1.3,supplyHpMul:1.5}:{hpMul:1},tag=variant?'duck':null;
 const b=new Battle({seed:42,content:'none',recordEvents:false,rect:{r0:0,r1:18,c0:0,c1:20},stage:{rows:Array(19).fill('r'.repeat(21)),legend:{}},data:{enemies:data},enemyOverrides:{[key]:overrides}});
 content.install(b);
 const e=b.spawnEnemy(key,{pos:[10,5.2],mods,tag});
 const a=new Unit({id:10000,side:'ally',kind:'op',x:5,y:10,base:{maxHp:1000,atk:120,blockCnt:3}});a.deployed=true;a.ground=true;
 b.units.push(a);b.allyUnits.push(a);b._occ[215]=a;
 const view=()=>{const s=e.s;return {hp:bits(e.hp),alive:e.alive,stealth:enemyStealthed(e),selectable:canTargetEnemy(a,e,{canHitFly:true}),blocked:e.blockedBy?.id??null,stats:Object.fromEntries(['maxHp','res','atk'].map(k=>[k,bits(s[k])])),flags:Object.keys(s.flags).filter(k=>s.flags[k]).sort(),buffs:e.buffs.map(x=>({key:x.key,left:bits(x.timeLeft),persist:!!x.persist,visible:!!x.visible,stacks:x.stacks,flags:Object.keys(x.flags??{}).filter(k=>x.flags[k]).sort(),mods:Object.fromEntries(Object.entries(x.mods??{}).map(([k,n])=>[k,bits(n)])),restore:x.flags?.stealth?bits(x.data?.stealthRestore??3):null})),rng:b.rng.state()};};
 const states=[view()],ops=[];
 for(let j=0;j<86;j++){
  b.time=j*.1;let op={time:b.time};
  if(j===3){op.half=true;e.hp*=.7;}
  if([4,31,61].includes(j)){op.status='silence';b.applyStatus(e,'silence',{duration:.4});}
  if([10,52].includes(j)){op.status='reveal';b.applyStatus(e,'reveal',{duration:.4});}
  if([14,28,48].includes(j)){op.block=true;b._checkBlock(e);}
  if([18,29,50].includes(j)){op.unblock=true;b._unblock(e);}
  if(j===25){op.extra=true;b.addBuff(e,{key:'otherStealth',duration:1.5,flags:{stealth:true},data:{stealthRestore:.3}});}
  if(j%13===11){op.hit=['phys','arts','true','elemental'][(j+variant)%4];b.dealDamage(a,e,{amount:5.7,type:op.hit,isAttack:true,noSp:true});}
  if(j===70){op.loss=true;b.loseHp(e,.3);}
  e.mem.carrier=j>=34&&j<37?a:null;op.carrier=!!e.mem.carrier;
  b._tickBuffs(.1);b.emit('tick',{dt:.1});
  ops.push(op);states.push(view());
 }
 console.log(JSON.stringify({key,raw:data[key],overrides,mods,tag,ops,states}));
}
