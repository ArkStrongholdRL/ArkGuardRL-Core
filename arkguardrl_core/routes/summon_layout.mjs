// GPL-3.0-or-later. Exposure objective follows Stronghold Protocol bot.js Layout.value.
// Engine remains external; public engine helpers supply routes, ranges and nominal DPS.
// This is a teacher objective, not a learned value function or game reward.
export function fixedAllyPlanner(bot, boardApi, match, player, pieces, oldPlan) {
  const gd = player.gd || match.gd;
  const model = bot.fieldModel(match, player);
  const params = bot.LAYOUT_PARAMS;
  const healer = rec => rec?.dmgType === 'heal' || rec?.attackKind === 'heal';
  const blocker = rec => boardApi.basePositionClass(rec) === 'melee'
    && (rec?.stats?.blockCnt ?? 1) > 0 && rec?.attackKind !== 'none';
  const record = p => p.kind === 'token' ? gd.token(p.id) : bot.rangeRec(player, gd.chess(p.id));
  const unit = (p, key, dir) => {
    const rec = record(p);
    const [r,c] = key.split(',').map(Number);
    return {key, block:blocker(rec) ? Math.max(1,rec.stats?.blockCnt ?? 1) : 0,
      heal:healer(rec), ground:!healer(rec) && rec.attackKind !== 'heal',
      air:!!rec.canHitFly && !healer(rec) && rec.attackKind !== 'none',
      dps:bot.effDps(model,rec)*bot.airflowAtkMul(model,key,dir),
      cover:new Set(bot.rangeTiles(rec,r,c,dir))};
  };
  const fixed = [...player.board].filter(([,p]) => p.kind === 'chess')
    .map(([key,p]) => unit(p,key,boardApi.pieceDir(p)));
  const score = units => {
    let total = 0;
    for (const route of model.routes) {
      let exposure = 0, lastBlock = -Infinity;
      for (let i=0;i<route.tiles.length;i++) {
        const key = route.tiles[i];
        let time = route.tileTime + (i<2 && route.dwell ? route.dwell : 0);
        const block = units.filter(u => u.key===key && model.ground.has(key)).reduce((n,u)=>n+u.block,0);
        if (!route.fly && block>0) {
          const healing = Math.min(2,units.filter(u=>u.heal && u.cover.has(key)).length);
          time += params.hold*block*(i-lastBlock>=params.spread ? 1 : params.secondHold)*(1+params.healHold*healing);
          lastBlock=i;
        }
        const damage = units.filter(u=>u.cover.has(key) && (route.fly ? u.air : u.ground)).reduce((n,u)=>n+u.dps,0);
        exposure += time*damage;
      }
      total += route.n*(1-Math.exp(-exposure/(params.kill*route.hp)));
    }
    for (const u of units) if (!u.block && model.ground.has(u.key)) total -= params.roadPenalty*model.ground.get(u.key).flow;
    return total;
  };
  const plan = new bot.LayoutPlan();
  const units = [...fixed];
  const taken = new Set(fixed.map(u=>u.key));
  const rank = p => blocker(record(p)) ? 0 : healer(record(p)) ? 2 : 1;
  const ordered = [...pieces].sort((a,b)=>rank(a)-rank(b)||bot.effDps(model,record(b))-bot.effDps(model,record(a))||a.uid-b.uid);
  for (const p of ordered) {
    const within = player.summonRange(p), outside = player.summonExcluded(p);
    let best = null, value = -Infinity;
    for (const [r,c] of boardApi.legalTiles(player.deployMap(),boardApi.positionClass(record(p)))) {
      const key = `${r},${c}`;
      if (taken.has(key) || within && !within.has(key) || outside && outside.has(key)) continue;
      for (const dir of bot.PLAN_DIRS) {
        const candidate = unit(p,key,dir), v = score([...units,candidate]);
        if (v>value) {best={key,dir,candidate};value=v;}
      }
    }
    if (!best) continue;
    plan.set(p.uid,best.key);plan.dirs.set(p.uid,best.dir);
    taken.add(best.key);units.push(best.candidate);
  }
  const oldUnits = pieces.filter(p=>oldPlan.has(p.uid)).map(p=>unit(p,oldPlan.get(p.uid),oldPlan.dirOf(p.uid)));
  return {plan, comparison:{objective:'fixed-allies public nominal exposure; teacher only',
    fixedAllies:fixed.length, baselineScore:score(fixed), oldPlanScore:score([...fixed,...oldUnits]),
    newPlanScore:score(units)}};
}
