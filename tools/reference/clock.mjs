// Real BattleLifecycle methods with instrumented phase bodies; no matches/data.
import path from'node:path';import{pathToFileURL}from'node:url';
const root=path.resolve(process.argv[2]);const{BattleLifecycle}=await import(pathToFileURL(path.join(root,'server/sim/battle/lifecycle.js')));
const phases=['start','scheduled','spawns','dp','buffs','attackCommit','enemies','enemyIndex','allies','rate','projectiles','redeploy','boss','tickHook','endCheck'];
for(let stop=-1;stop<phases.length;stop++)for(const reenter of [false,true])for(const hook of [false,true]){
 const trace=[];const b={finished:false,_stepping:false,started:false,_endReq:null,time:0,tickCount:0,dt:1/30,_toRelease:[1],_hooks:hook?{tick:true}:{},players:[],enemies:[],allyUnits:[],flags:{},projectiles:{update(){}},_phase(name,fn){if(this.finished||this._endReq)return;trace.push(name);if(reenter)BattleLifecycle.prototype.step.call(this);if(name==='start')this.started=true;if(phases[stop]===name)this._endReq='requested';},_compactEnemies(){trace.push('compact');},_releaseRemoved(){trace.push('release');this._toRelease=[];},forceEnd:BattleLifecycle.prototype.forceEnd,_finish(reason){trace.push('finish:'+reason);this.finished=true;this.reason=reason;},_timeout(){this._finish('timeout');}};
 for(let i=0;i<3;i++)BattleLifecycle.prototype.step.call(b);
 process.stdout.write(JSON.stringify({stop,reenter,hook,time:b.time,ticks:b.tickCount,finished:b.finished,trace})+'\n');
}
for(const shared of [null,-1,1])for(const boss of [false,true])for(const auto of [false,true])for(const pending of [false,true])for(const enemy of [false,true])for(const hold of [false,true])for(const timeout of [false,true]){
 let reason=null;const b={finished:false,sharedBoss:shared===null?null:{hp:shared},kind:boss?'boss':'normal',autoFinish:auto,_pending:pending?[1]:[],started:true,_anyEnemyAlive:()=>enemy,_hasPendingEnemySchedules:()=>hold,time:1/30,timeLimit:timeout?0:10,_finish(r){reason=r;},_timeout(){reason='timeout';}};
 BattleLifecycle.prototype._checkEnd.call(b);
 process.stdout.write(JSON.stringify({policy:true,shared,boss,auto,pending,enemy,hold,timeout,reason})+'\n');
}
