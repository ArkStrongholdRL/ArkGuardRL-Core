// Target filters, sort keys and pre-movement attack commit gates, without matches/data.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const t=await imp('server/sim/targeting.js');const ai=await imp('server/sim/ai.js');const{createRng}=await imp('server/sim/rng.js');const rng=createRng(52893);
const priorities=[null,'fly','lowDef','highDef','ranged','lowestHp','highestHp','lowestHpRatio','highestAtk','boss','elite','notBurst','ground','heaviest','nearest','farthest'];
for(let i=0;i<512;i++){
 const flags=['sleep','untargetable','stealth','reveal','stealthOff','liftoff','camou','groundbind','levitate','float','burstLock'];
 const records=Array.from({length:6},(_,j)=>({id:j+1,side:j===0?'ally':'enemy',flags:Object.fromEntries(flags.filter(()=>rng()<.15).map(k=>[k,true])),hp:10+rng()*90,base:{maxHp:100,atk:rng()*100,def:rng()*100,res:rng()*100,massLevel:rng()*5,rangeRadius:j%2},spawn:1+j%3,aggro:j%3,motion:j%2?'FLY':'WALK',elite:j%3===0,boss:j%4===0,apply:j%2?'MELEE':'RANGED',hidden:rng()<.05,alive:rng()>.05,deployed:rng()>.05,remaining:j%3?rng()*30:null,x:4+rng(),y:10+rng()}));
 const units=records.map(d=>{const u=new Unit({id:d.id,side:d.side,kind:'op',x:d.x,y:d.y,base:d.base,motion:d.motion});u.buffs=[{key:'stealthA',stacks:1,mods:{},flags:d.flags,shield:0}];u.hp=d.hp;u.alive=d.alive;u.deployed=d.deployed;u.hidden=d.hidden;u.spawnSeq=d.spawn;u.aggroSeq=d.aggro;u.deploySeq=d.id;u.isBoss=d.boss;u.def={rank:d.elite?'ELITE':'NORMAL',applyWay:d.apply};u.findBuff=()=>null;return u;});
 const profile={hitSleep:i%3===0,canHitFly:i%2===0,dollHitsFly:i%5===0,groundOnly:i%7===0,blockFree:i%11===0,skipEnemy:e=>e.id===6&&i%13===0};units[0].trait.doll=i%2===1;units[1].blockedBy=units[0];units[0].blocking=[units[1]];
 const battle={remainingDistance:e=>records[e.id-1].remaining??Infinity};
 const enemyOrder=priorities.map(priority=>t.sortEnemyTargets(battle,units[0],units.slice(1),priority).map(u=>u.id));
 const allyOrder=t.sortAllyTargets(Object.assign(units[1],{profile}),[units[4],units[0],units[3],units[2]]).map(u=>u.id);
 const filters=units.slice(1).map(e=>({id:e.id,enemy:t.canTargetEnemy(units[0],e,profile),ally:t.canTargetAlly(units[1],e,true),aura:t.auraSelectable(units[1],e),area:t.areaSelectable(units[1],e)}));
 const grid=[[0,0],[1,1],[0,1],[1,-1],[0,0]];const ranges=['UP','RIGHT','DOWN','LEFT'].map(dir=>t.absoluteRangeKeys(grid,10,4,dir,i%24));
 process.stdout.write(JSON.stringify({i,records,profile:{...profile,skip:i%13===0},doll:units[0].trait.doll,enemyOrder,allyOrder,filters,ranges})+'\n');
}
for(let i=0;i<1024;i++){
 const cd=[0,-1,1/30,1/30+1e-9,1/30+2e-9,.5,1][i%7],act=i%3!==0,disarm=i%5===0,active=i%7===0,had=i%11===0?false:undefined;
 const profile={noAttack:i%13===0,noAttackUnlessSkill:i%17===0,heal:i%19===0?{}:null,dmgType:i%19===0?'heal':'phys',allInRange:i%23===0,rangeAoe:i%29===0};
 const u=new Unit({id:1,side:'ally',kind:'op'});u.alive=act;u.deployed=true;u.atkCd=cd;u.profile=profile;u.skill={active,attackOverride:()=>null,targetingOverride:()=>null};u.trait.hadTarget=had;u.buffs=[{stacks:1,flags:{disarm},mods:{},shield:0}];const enemy=new Unit({id:2,side:'enemy',kind:'enemy'});enemy.deployed=true;
 const b={allyUnits:[u],enemiesInKeys:()=>[enemy],remainingDistance:()=>1};ai.noteAttackCommit(b,1/30);
 process.stdout.write(JSON.stringify({cadence:true,cd,act,disarm,active,had:had??null,profile,commit:!!b._attackCommit?.has(1),countdown:ai.attackCountdown(cd,1/30)})+'\n');
}
