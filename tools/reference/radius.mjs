// Stronghold Protocol 0.3.3 actual radius selectors; no assets or match engine.
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const root=path.resolve(process.argv[2]);
const {BattleQueries}=await import(pathToFileURL(path.join(root,'server/sim/battle/queries.js')));
const {Unit}=await import(pathToFileURL(path.join(root,'server/sim/units.js')));
const {normHitArea}=await import(pathToFileURL(path.join(root,'server/sim/body.js')));
for(let i=0;i<256;i++){
 const records=Array.from({length:12},(_,j)=>({id:j+1,x:4+(j%4)*.6,y:10+Math.floor(j/4)*.49,huge:j%3===0,owner:j%2+1,device:j===2,alive:!(i%5===0&&j===3),deployed:!(i%7===0&&j===4),hidden:i%11===0&&j===5,stealth:(i+j)%7===0,untargetable:(i+j)%11===0,revealed:(i+j)%3===0}));
 const units=side=>records.map(d=>{const u=new Unit({id:d.id,side,kind:d.device?'device':'enemy',x:d.x,y:d.y});u.alive=d.alive;u.deployed=d.deployed;u.hidden=d.hidden;u.ownerId=d.owner;u.hitArea=d.huge?normHitArea({w:4.95,h:2.95,dx:0,dy:1}):null;u.buffs=[{mods:{},flags:{stealth:d.stealth,reveal:d.revealed,untargetable:d.untargetable},stacks:1,shield:0}];return u;});
 const b={enemies:units('enemy'),allyUnits:units('ally'),enemiesInRadius:BattleQueries.prototype.enemiesInRadius,foesInRadius:BattleQueries.prototype.foesInRadius};
 const radius=[0,.1,.4999999999,1,2,-.1][i%6],owner=i%2?1:null,devices=i%3===0;
 const ids=x=>x.map(u=>u.id);
 process.stdout.write(JSON.stringify({i,records,radius,owner,devices,centre:ids(b.enemiesInRadius(4,10,radius,true)),body:ids(b.enemiesInRadius(4,10,radius)),foes:ids(b.foesInRadius(4,10,radius)),collider:ids(BattleQueries.prototype.foesInColliderRadius.call(b,4,10,radius)),allies:ids(BattleQueries.prototype.alliesInRadius.call(b,4,10,radius,owner,{includeDevices:devices}))})+'\n');
}
