// Real upstream block contact/release functions; no matches/data.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{Grid}=await imp('server/sim/grid.js');const{BattleBlocking}=await imp('server/sim/battle/blocking.js');
for(let i=0;i<1024;i++){
 const d={i,fly:i%3===0,blockFly:i%4===0,profileFly:i%13===0,ground:i%17!==0,fence:i%19===0,sleep:i%23===0,unblockable:i%29===0,hidden:i%31===0,device:i%7===0,cap:i%4,weight:1+i%2,radius:(i%3)*.23,x:4+(i%11-5)/10,y:10+(Math.floor(i/11)%11-5)/10};
 const stage={rows:Array(19).fill('r'.repeat(21))};if(d.fence)stage.rows[10]=stage.rows[10].slice(0,4)+'b'+stage.rows[10].slice(5);
 const grid=new Grid(stage,{r0:0,r1:18,c0:0,c1:20});const occ=Array(399).fill(null);const units=[];
 for(const [id,row,col]of[[1,10,4],[2,10,5]]){const u=new Unit({id,kind:d.device?'device':'op',side:'ally',tileR:row,tileC:col,base:{blockCnt:d.cap}});u.deployed=true;u.ground=d.ground;u.profile={blockFly:d.profileFly};u.buffs=[{stacks:1,mods:{blockRadiusScale:d.radius},flags:{blockFly:d.blockFly,sleep:d.sleep}}];occ[row*21+col]=u;units.push(u);}
 const enemy=new Unit({id:3,side:'enemy',kind:'enemy',x:d.x,y:d.y,motion:d.fly?'FLY':'WALK'});enemy.blockWeight=d.weight;enemy.hidden=d.hidden;enemy.buffs=[{stacks:1,flags:{unblockable:d.unblockable}}];enemy.moving=true;
 const trace=[];const b={grid,_occ:occ,_hooks:{blocked:true},emit(n,e){trace.push([n,e.blocker.id,e.enemy.id]);},_stealthSwitch(e){trace.push(['ended',e.id]);},_blockerFor:BattleBlocking.prototype._blockerFor};
 const checks=[];checks.push(BattleBlocking.prototype._checkBlock.call(b,enemy));checks.push(BattleBlocking.prototype._checkBlock.call(b,enemy));const held=enemy.blockedBy?.id??null;
 if(i%2)BattleBlocking.prototype._unblock.call(b,enemy);else for(const u of units)BattleBlocking.prototype.releaseBlocked.call(b,u);
 checks.push(BattleBlocking.prototype._checkBlock.call(b,enemy));
 process.stdout.write(JSON.stringify({scenario:d,checks,held,final:enemy.blockedBy?.id??null,lists:units.map(u=>u.blocking.map(e=>e.id)),moving:enemy.moving,trace})+'\n');
}
