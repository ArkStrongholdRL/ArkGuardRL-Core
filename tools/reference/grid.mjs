// Development-only geometry oracle. Reads external stage data; no match or training.
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const root=path.resolve(process.argv[2]);
const {Grid,bresenhamTiles,segmentTiles,straightClear}=await import(pathToFileURL(path.join(root,'server/sim/grid.js')));
const dir=await import(pathToFileURL(path.join(root,'server/sim/dir.js')));
const {BattlePlayers}=await import(pathToFileURL(path.join(root,'server/sim/battle/players.js')));
const out=x=>process.stdout.write(JSON.stringify(x)+'\n');
for(const d of dir.DIRS) for(let r=-3;r<=3;r++) for(let c=-3;c<=3;c++)
  out({kind:'direction',d,r,c,rotated:dir.rotateOffset(r,c,d),local:dir.toLocal(r,c,d),mirror:dir.mirrorDir(d),opposite:dir.oppositeDir(d),delta:dir.dirFromDelta(r,c,d)});
for (const kind of ['normal','boss','hidden']) for(const side of ['L','R']) for(const field of [false,true]) for(const abs of [false,true]) for(const offset of [null,-2]) {
  const context={kind,_boardTile:BattlePlayers.prototype._boardTile};
  const ps={mirror:kind!=='normal'&&side==='R',coords:field?'field':'board',rowOffset:offset,colOffset:side==='R'?8:0};
  for(const p of [[9,2],[12,10],[2,18],[0,0]])
    out({kind:'placement',battle:kind,side,field,abs,offset,p,tile:BattlePlayers.prototype.mapTile.call(context,ps,...p,abs),dir:BattlePlayers.prototype.mapDir.call(context,ps,'RIGHT',abs)});
}
for(let r=0;r<19;r+=3) for(let c=0;c<21;c+=3) {
  const a=[r,c],b=[18-r,20-c];out({kind:'line',a,b,bresenham:bresenhamTiles(a,b),segment:segmentTiles(a,b)});
}
const stages=JSON.parse(fs.readFileSync(path.join(root,'data/stages.json'),'utf8'));
const rects=[{r0:0,r1:18,c0:0,c1:20},{r0:9,r1:12,c0:2,c1:10},{r0:0,r1:5,c0:0,c1:20}];
for(const [id,raw] of Object.entries(stages)) {
  const stage={...raw,legend:raw.tiles??raw.legend};
  for(const rect of rects) for(const obstacles of [false,true]) {
    const g=new Grid(stage,rect);
    if(obstacles){g.setObstacle(9,8,true,'crate');g.setObstacle(10,8,true);g.setObstacle(9,8,true);g.setObstacle(9,8,false);}
    const queries=[];
    for(let r=0;r<19;r++)for(let c=0;c<21;c++)queries.push([g.groundPassable(r,c),g.walkable(r,c),g.flyPassable(r,c),g.canStand(r,c),g.canStand(r,c,{ranged:true})]);
    const tiles=g.tiles.map(t=>[t.glyph,t.key,t.height,t.build,t.pass,t.terrain??null,t.special??null]);
    out({kind:'grid',id,rect,obstacles,tiles,queries,version:g.version});
    for (const end of [[9,2],[2,18],[12,10]]) for (const start of [[9,10],[12,10],[2,2]]) for(const delta of [[0,0],[.25,-.25],[.5,.5],[-.5,-.5]]) {
      const x=start[1]+delta[1],y=start[0]+delta[0];
      out({kind:'straight',x,y,end,value:straightClear(g,x,y,{x:end[1],y:end[0]})});
    }
    for(const end of [[9,2],[2,2],[2,18],[-1,0]])for(const diagonal of [true,false])for(const ignore of [false,true]) {
      const f=g.flowField(...end,{allowDiagonal:diagonal,ignoreObstacles:ignore});
      const fields=Object.fromEntries(['dist','parent','pen','next','official','cost','deep'].map(k=>[k,Array.from(f[k])]));
      const paths=[[9,10],[12,10],[2,18],[2,2]].map(start=>({start,waypoints:g.waypoints(...start,...end,{allowDiagonal:diagonal,ignoreObstacles:ignore}),length:Number.isFinite(g.fieldLength(f,start[0]*21+start[1]))?g.fieldLength(f,start[0]*21+start[1]):null}));
      out({kind:'flow',id,rect,obstacles,end,diagonal,ignore,paths,...fields});
    }
  }
}
