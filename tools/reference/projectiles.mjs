// Real ProjectileSystem with explicit flight/impact callback fixtures.
import path from'node:path';import{pathToFileURL}from'node:url';const root=path.resolve(process.argv[2]);const imp=p=>import(pathToFileURL(path.join(root,p)));
const{Unit}=await imp('server/sim/units.js');const{ProjectileSystem}=await imp('server/sim/projectiles.js');const bits=x=>{const b=Buffer.alloc(8);b.writeDoubleBE(x);return b.toString('hex');};
for(let i=0;i<128;i++){
 const d={i,hitDead:i%2===0,speed:[0,2,12][i%3],maxAge:i%5===0?.2:10,die:i%7===0,hide:i%11===0,redeploy:i%13===0,cancel:i%17===0,spawnStep:i%3===0,spawnHit:i%5===0};
 const source=new Unit({id:1,side:'ally'}),target=new Unit({id:2,side:'enemy',x:2,y:1});target.deploySeq=1;target.deployed=true;const trace=[];let sys;let spawned=false;
 const impact=c=>{trace.push(['impact',c.projectile.data,c.target?.id??null,bits(c.x),bits(c.y)]);if(d.spawnHit&&c.projectile.data<10)sys.add({from:{x:c.x,y:c.y},to:{x:0,y:0},speed:12,data:c.projectile.data+100,onHit:impact});};
 const b={_hooks:{projectileStep:true},emit(n,c){const p=c.projectile;trace.push(['step',p.data,bits(p.x),bits(p.y)]);if(d.spawnStep&&!spawned){spawned=true;sys.add({from:{x:p.x,y:p.y},to:{x:3,y:2},speed:12,data:10,onHit:impact});}if(d.cancel&&p.data===2)c.cancel=true;},_safe:fn=>fn()};sys=new ProjectileSystem(b);
 for(let token=1;token<=2;token++)sys.add({from:source,target,speed:d.speed,hitDead:d.hitDead,maxAge:d.maxAge,data:token,onHit:impact});
 const states=[];for(let tick=0;tick<30;tick++){target.x=2+tick*.03;target.y=1+tick*.01;if(d.die&&tick===2)target.alive=false;if(d.hide&&tick===2)target.hidden=true;if(d.redeploy&&tick===3){target.alive=true;target.hidden=false;target.deploySeq++;}sys.update(.1);states.push(sys.list.map(p=>({token:p.data,x:bits(p.x),y:bits(p.y),tx:bits(p.tx),ty:bits(p.ty),age:bits(p.age),target:p.target?.id??null})));}
 process.stdout.write(JSON.stringify({scenario:d,states,trace})+'\n');
}
