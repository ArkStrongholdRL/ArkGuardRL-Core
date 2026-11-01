// Real collider helpers and BattleQueries index; no match or assets.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const body=await imp('server/sim/body.js');const{Unit}=await imp('server/sim/units.js');const{BattleQueries}=await imp('server/sim/battle/queries.js');const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<1024;i++){
 const x=(i%31-5)/2+[0,.4,.4999999999,.5000000001][i%4],y=(Math.floor(i/31)%25-3)/2;
 const area=i%3===0?{w:4.95,h:2.95,dx:i%2?.3:0,dy:1}:i%7===0?{w:1,h:1,dx:0,dy:0}:null;
 const u={x,y,side:i%2?'enemy':'ally',hitArea:body.normHitArea(area)};const origin=[10,4];const tiles=[214,215,235,236,237];
 process.stdout.write(JSON.stringify({body:true,i,x,y,area,side:u.side,keys:body.bodyKeys(u),collider:body.colliderKeys(u),on:body.bodyOnTile(u,10,4),inKeys:body.bodyInKeys(u,tiles),inRange:body.bodyInRange(u,tiles),distance:bits(body.bodyDist(u,origin[1],origin[0])),radius:body.bodyInRadius(u,origin[1],origin[0],2),reach:bits(body.bodyTileReach(u,10,4))})+'\n');
}
for(let i=0;i<128;i++){
 const records=Array.from({length:6},(_,j)=>({id:j+2,x:4.4+(j%3)*.2,y:10.49+Math.floor(j/3)*.3,huge:j%3===0,hidden:i%7===0&&j===2,alive:!(i%5===0&&j===3),deployed:!(i%11===0&&j===1),fly:j%2===1,sleep:i%13===0&&j===0,untargetable:i%17===0&&j===4}));
 const enemies=records.map(d=>{const u=new Unit({id:d.id,side:'enemy',kind:'enemy',x:d.x,y:d.y,motion:d.fly?'FLY':'WALK'});u.alive=d.alive;u.deployed=d.deployed;u.hidden=d.hidden;u.hitArea=d.huge?body.normHitArea({w:4.95,h:2.95,dx:0,dy:1}):null;u.buffs=[{stacks:1,mods:{},flags:{sleep:d.sleep,untargetable:d.untargetable},shield:0}];return u;});
 const b={enemies,_eb:Array.from({length:399},()=>[]),_et:Array.from({length:399},()=>[]),_ebUsed:[],_etUsed:[],enemiesOnTiles:BattleQueries.prototype.enemiesOnTiles};BattleQueries.prototype._buildEnemyIndex.call(b);
 const attacker=new Unit({id:1,side:'ally'});attacker.profile={rangeTiles:i%2===0};const p={canHitFly:i%3===0,hitSleep:i%5===0,skipEnemy:e=>i%7===0&&e.id===3};
 const keys=[214,235,215,236,214,999];const ids=list=>list.map(u=>u.id);
 process.stdout.write(JSON.stringify({index:true,i,records,rangeTiles:attacker.profile.rangeTiles,fly:p.canHitFly,hitSleep:p.hitSleep,skip:i%7===0,keys,enemies:ids(BattleQueries.prototype.enemiesInKeys.call(b,keys,attacker,p)),tiles:ids(BattleQueries.prototype.enemiesOnTiles.call(b,keys,attacker,p)),any:BattleQueries.prototype.anyEnemyInKeys.call(b,keys,attacker)})+'\n');
}
for(let i=0;i<256;i++){
 const records=Array.from({length:9},(_,j)=>({id:j+1,tile:[10,j%2?6:8],hp:[100,1000-5e-7,1000][(i+j)%3],maxHp:1000,seq:(j*7+i)%9,alive:!(i%5===0&&j===3),deployed:!(i%7===0&&j===2),hidden:i%11===0&&j===4,device:j===5,noHeal:(i+j)%3===0,profileNoHeal:i%13===0&&j===1,element:j===6?1:0}));
 const allies=records.map(d=>{const u=new Unit({id:d.id,side:'ally',kind:d.device?'device':'op',x:d.tile[1],y:d.tile[0],base:{maxHp:d.maxHp}});u.hp=d.hp;u.deploySeq=d.seq;u.alive=d.alive;u.deployed=d.deployed;u.hidden=d.hidden;u.profile={noHeal:d.profileNoHeal};u.buffs=[{mods:{},flags:{noHeal:d.noHeal},stacks:1,shield:0}];u.elem.burn=d.element;return u;});
 const healer=i%2?allies[1]:null;const through=i%3===0;if(healer)healer.profile.healThrough=()=>through;
 const b={allyUnits:allies};const keys=[216,218];
 process.stdout.write(JSON.stringify({healing:true,i,records,keys,healer:healer?.id??null,through,includeElement:i%2===0,targets:BattleQueries.prototype.injuredAlliesInKeys.call(b,keys,healer,i%2===0).map(u=>u.id)})+'\n');
}
