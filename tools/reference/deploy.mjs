// Direct upstream deployment methods with explicit skill/content stubs. No matches/data.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{BattleDeploy}=await imp('server/sim/battle/deploy.js');const{BattleTiles}=await imp('server/sim/battle/tiles.js');const{Grid}=await imp('server/sim/grid.js');
const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<384;i++){
 const d={i,token:i%5===0,foreign:i%3===0,homeTaken:i%7===0,downHome:i%11===0,raid:i%13===0,permanent:i%17===0,revive:i%19===0,retreat:i%2===0};
 const trace=[];const grid=new Grid({rows:Array(19).fill('r'.repeat(21))},{r0:9,r1:12,c0:2,c1:10});
 const make=(id,kind,tile)=>{const u=new Unit({id,uid:id,side:'ally',kind,tileR:tile[0],tileC:tile[1],base:{maxHp:100,cost:7,respawnTime:3}});u.skill={active:false,reset(){},setSpTotal(){}};u.alive=false;return u;};
 const a=make(1,d.token?'token':'op',[9,2]),b=make(2,'op',d.foreign?[10,4]:[12,10]);a.ownerId='p';b.ownerId='p';
 const ps={dp:20};const btl={time:0,grid,_occ:Array(399).fill(null),_deploySeq:0,allyUnits:[a,b],units:[a,b],flags:{startOpCooldown:3,dpMax:99},_hooks:{kill:true,death:true,deploy:true},_toRelease:[],kind:'normal',_startDeploying:false,
 log(){},_ev(){},_refreshRange(){},_cutAttackStand(){},releaseBlocked(){},_unblock(){},_pp(){return {deaths:0};},getPlayer(){return ps;},
 emit(name,e){trace.push([name,e.unit?.id??e.victim.id]);if(name==='kill'&&d.revive)e.victim.hp=15;},_safe(fn){fn();}};
 for(const name of ['_deploy','kill','retreat','_remove','redeploy','_checkRedeploys'])btl[name]=BattleDeploy.prototype[name];
 for(const name of ['isDown','restTile','downOn','isReservedTile','_layBody','relocate'])btl[name]=BattleTiles.prototype[name];
 btl._deploy(a);btl._deploy(b);
 if(d.foreign)btl.relocate(b,12,9);
 btl.relocate(a,10,4);a.downAtHome=d.downHome;
 if(d.homeTaken)btl.relocate(b,9,2);
 btl.time=1;
 if(d.retreat)btl.retreat(a,{reason:d.raid?'raid':'retreat',permanent:d.permanent});else btl.kill(a,null);
 const before={alive:a.alive,deployed:a.deployed,removed:a.removed,hp:bits(a.hp),down:btl.isDown(a),body:a.body,rest:btl.restTile(a),respawn:bits(a.respawnAt)};
 btl.time=5;btl._checkRedeploys();
 const after={alive:a.alive,deployed:a.deployed,removed:a.removed,hp:bits(a.hp),down:btl.isDown(a),body:a.body,rest:btl.restTile(a),respawn:bits(a.respawnAt),seq:a.deploySeq,tile:[a.tileR,a.tileC],dp:bits(ps.dp)};
 process.stdout.write(JSON.stringify({scenario:d,before,after,trace})+'\n');
}
