// Standalone experimental extension of the native bridge.
// Independent native-intent bridge for the frozen Stronghold Protocol 0.3.1 engine.
// Engine implementation stays external; no legacy ArkGuardRL adapter is imported.
import fs from 'node:fs';
import {geometry, captureTeacher, ROUTE_SCHEMA} from './route_teacher.mjs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import readline from 'node:readline';

if (Number(process.versions.node.split('.')[0]) < 22) throw new Error('Node 22+ required');

const REVISION = 'c9d804f470b8d7422ae04eb66faf671cef21ad52';
const SCHEMA = 'arkguardrl/native-session-v2';
const PLAYER = 'agent';
const root = fs.realpathSync(process.argv[2]);
const git = (...args) => execFileSync('git', ['-C', root, ...args], { encoding: 'utf8' }).trim();
if (git('rev-parse', 'HEAD') !== REVISION) throw new Error('Expected frozen 0.3.1 engine revision');
if (git('status', '--porcelain', '--untracked-files=all')) throw new Error('Engine checkout must be clean');
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
if (pkg.version !== '0.3.1') throw new Error('Expected engine app version 0.3.1');
const digest = createHash('sha256');
for (const name of fs.readdirSync(path.join(root, 'data')).filter(n => n.endsWith('.json')).sort()) {
  digest.update(name + '\0');
  digest.update(fs.readFileSync(path.join(root, 'data', name)));
  digest.update('\0');
}
const identity = { schema: SCHEMA, engineRevision: REVISION, appVersion: pkg.version, dataSha256: digest.digest('hex') };
const imp = rel => import(pathToFileURL(path.join(root, rel)).href);
const [{ Match }, { VirtualScheduler }, { getData, DATA_FILES }, protocol, constants] = await Promise.all([
  imp('server/match/Match.js'), imp('server/match/scheduler.js'), imp('server/data.js'), imp('shared/protocol.js'), imp('shared/constants.js'),
]);
const bot = await imp('server/match/bot.js');
const quiet = { info() {}, debug() {}, warn() {}, error() {} };
const data = getData({ log: quiet });
for (const key of DATA_FILES) {
  if (!data[key] || !Object.keys(data[key]).length) throw new Error(`Missing or empty engine data: ${key}`);
}
let match = null, scheduler = null, revision = 0, summary = null;
const PUBLIC_KEYS = ['t', 'phase', 'round', 'lastRound', 'deadline', 'serverNow', 'modeId', 'difficulty',
  'stageId', 'setupRevision', 'rerollVote', 'factions', 'disabledBonds', 'drawnDisabledBonds', 'bannedChess',
  'bossId', 'bossRound', 'hiddenRound', 'bossPairs', 'spRound', 'combatMode', 'paused', 'players', 'fields',
  'offers', 'teamLp', 'overtimeAt', 'bossHp', 'draft', 'sp', 'unite', 'uniteResult'];
const OWN_KEYS = ['t', 'playerId', 'seat', 'alive', 'lp', 'funds', 'bandId', 'ready', 'canReady',
  'personalChoice', 'shop', 'hand', 'temp', 'board', 'prepMapChars', 'deployCap', 'deployCount',
  'bonds', 'effects', 'nextEnemies', 'loadout', 'ops', 'standIns', 'diy', 'diyBanned', 'bossMate', 'stats'];
const PREVIEW_KEYS = ['enemyKey', 'count', 'gate', 't', 'fly', 'elite', 'boss', 'source', 'tag', 'start'];
const pick = (value, keys) => Object.fromEntries(keys.filter(k => Object.hasOwn(value, k)).map(k => [k, value[k]]));
// Seeds are configuration/provenance, never a policy observation. Do not export battle specifications.
function visible(value) {
  if (Array.isArray(value)) return value.map(visible);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !/seed|rng|hiddenBossId/i.test(key)).map(([key, v]) => [key, visible(v)]));
  return value;
}
function close() {
  match?.dispose(); scheduler?.dispose(); match = null; scheduler = null;
}
function playerDecisionPending() {
  const ps = match.players.get(PLAYER);
  if (match.ended || !ps.alive || ps.left) return false;
  return (match.phase === 'INFO_CHECK' && !ps.infoReady)
    || (match.phase === 'BAND_DRAFT' && !ps.bandId)
    || (match.phase === 'PREP' && (!ps.ready || !!ps.personalChoice))
    || (match.phase === 'SP_DRAFT' && match.sp?.picks[PLAYER] == null);
}
function assertEngineHealthy() {
  if (!match) throw new Error('Reset required');
  if (match.errorCount || scheduler.errors.length || match.simErrors || match.dispatcher?.errors) throw new Error('Engine failure');
}
function observe() {
  assertEngineHealthy();
  const ps = match.players.get(PLAYER);
  const gameEnded = !!match.ended;
  const own = pick(ps.privateView(), OWN_KEYS);
  own.nextEnemies = own.nextEnemies.map(row => pick(row, PREVIEW_KEYS));
  const field = match.deployFieldOf(ps);
  own.deployment = { field, cells: [...match.deployMapFor(ps, field)].map(([key, kind]) => {
    const [row, col] = key.split(',').map(Number);
    return { row, col, kind };
  }).sort((a, b) => a.row - b.row || a.col - b.col) };
  return {
    schema: SCHEMA, revision, phase: match.phase, round: match.round,
    gameEnded, personalEnded: gameEnded || !ps.alive || !!ps.left,
    decisionRequired: playerDecisionPending(), clockMs: scheduler.now(), public: visible(pick(match.publicView(), PUBLIC_KEYS)), own: visible(own),
    outcome: gameEnded ? visible(summary) : null,
  };
}
function stateReply(extra = {}) {
  const state = observe();
  // Geometry belongs to this exact decision revision, not an extra round trip.
  return { ...extra, state, ...(state.decisionRequired && !state.personalEnded
    ? { geometry: geometry(match, match.players.get(PLAYER), bot, revision) } : {}) };
}
const ALLOWED = new Set(['g.infoReady', 'g.band', 'g.bandFocus', 'g.bandSkip', 'g.buy', 'g.refresh',
  'g.freeze', 'g.levelUp', 'g.sell', 'g.move', 'g.equip', 'g.art', 'g.destroy', 'g.unequip',
  'g.reward', 'g.choice', 'g.ready', 'g.offer', 'g.offerRefresh']);
function dispatch(req) {
  switch (req.cmd) {
    case 'identity': return { identity };
    case 'reset': {
      const seed = req.seed ?? 1;
      if (!Number.isInteger(seed) || seed < 1 || seed > 0xffffffff) throw new Error('seed must be uint32 > 0');
      const difficulty = req.difficulty ?? 'ABYSS';
      if (!constants.DIFFICULTIES.includes(difficulty)) throw new Error('Unsupported difficulty');
      close(); revision = 0; summary = null;
      scheduler = new VirtualScheduler({ start: 0, instantCombat: false, onError() {} });
      try {
        match = new Match({ roomCode: 'ARKGUARD', seed, mode: 'solo', difficulty, data, scheduler,
          clientCombat: false, botRehearsal: 0, botRehearsalDelegate: false,
          seats: [{ seat: 0, playerId: PLAYER, name: 'Agent', isBot: false, connected: true }],
          send() { return true; }, broadcast() {}, onEnd(result) { summary = result; }, log: quiet });
        match.start();
        return stateReply();
      } catch (err) { close(); throw err; }
    }
    case 'observe': return stateReply();
    case 'routeGeometry': {
      assertEngineHealthy();
      return {geometry:geometry(match,match.players.get(PLAYER),bot,revision)};
    }
    case 'layoutGoal': {
      assertEngineHealthy();
      const ps = match.players.get(PLAYER);
      if (match.phase !== 'PREP' || ps.ready) throw Error('Layout requires active PREP');
      const ids = req.uids;
      if (!Array.isArray(ids) || ids.length > 12 || new Set(ids).size !== ids.length)
        throw Error('Distinct bounded layout members required');
      const board = [...ps.board.values()];
      if (req.includeReserves !== undefined && typeof req.includeReserves !== 'boolean')
        throw Error('Explicit owned/deployed layout scope required');
      const available = req.includeReserves ? ps.allChess() : board;
      const pieces = ids.map(uid => available.find(p => p.uid === uid && p.kind === 'chess'));
      if (pieces.some(p => !p)) throw Error('Layout members outside requested public inventory scope');
      // Invoke the existing planner on an isolated read-only facade. Current
      // spawn identities/counts/routes are visible; private stat multipliers and
      // boss parts are deliberately omitted. No pool or future wave is queried.
      const sanitize = wave => wave ? {...wave, spawns: (wave.spawns || [])
        .filter(s => s.tag !== 'part').map(s => ({enemyKey:s.enemyKey,
          count:s.count, routeIndex:s.routeIndex, tag:s.tag}))} : null;
      const group = match.wave ? null : match.bossGroupOf(ps);
      const view = Object.create(match);
      view.wave = sanitize(match.wave);
      view.bossGroupOf = () => group ? {...group, wave:sanitize(group.wave)} : null;
      view._botPath = null;
      view.rngBots = () => .5; // Deterministic tie handling; no real RNG draw.
      const player = Object.create(ps);
      player.m = view;
      player._botRangeRecs = new Map();
      const occupied = new Set([...ps.board].filter(([,p]) => !ids.includes(p.uid)
        && (!req.includeReserves || p.kind !== 'chess')).map(([key]) => key));
      const plan = bot.planLayout(view, player, pieces, bot.LAYOUT_PARAMS, {occupied});
      return {layout:{schema:'arkguardrl/public-engine-layout/v1', revision,
        positions:[...plan].map(([uid,key]) => ({uid, cell:key.split(',').map(Number), dir:plan.dirOf(uid)})),
        missing:ids.filter(uid => !plan.has(uid)), tie:'constant-half',
        enemy_stats:'public nominal; private spawn multipliers omitted'}};
    }
    case 'teacherTakeover': {
      const ps=match.players.get(PLAYER); const before=observe();
      if(before.personalEnded)throw Error('Cannot correct ended game');
      const records=[];let bytes=0;
      const cap=captureTeacher(match,ps,()=>{revision++;return observe()},()=>geometry(match,ps,bot,revision),r=>{
        bytes+=Buffer.byteLength(JSON.stringify(r));if(bytes>32*1024*1024||records.length>=1024)throw Error('Teacher capture budget exceeded');records.push(r);
      });
      const start=performance.now();
      try {
        match.setAutoplay(ps,true);
        const complete=scheduler.runUntil(()=>summary!==null||performance.now()-start>60000,{maxSteps:5000000});
        observe();
        if(!complete||!summary||cap.rehearsals)throw Error('Incomplete/rehearsed teacher continuation');
        return {records,summary,rehearsals:cap.rehearsals,handoff:before};
      } finally {cap.restore()}
    }
    case 'act': {
      assertEngineHealthy();
      if (!req.action || !ALLOWED.has(req.action.t)) throw new Error('Unsupported native player intent');
      const error = protocol.validateC2S(req.action);
      if (error) throw new Error(`Invalid intent: ${error}`);
      const result = match.handle(PLAYER, req.action);
      revision++;
      return stateReply({ accepted: !!result?.ok, result: visible(result) });
    }
    case 'advance': {
      assertEngineHealthy();
      const phase = match.phase, round = match.round;
      const steps = req.maxSteps ?? 256;
      if (!Number.isInteger(steps) || steps < 1 || steps > 4096) throw new Error('maxSteps must be 1..4096');
      // Explicit clock progression only outside human decision phases. No invented solo deadlines.
      let executed = 0;
      if (!match.ended && !match.paused && !playerDecisionPending()) {
        while (executed < steps && scheduler.nextAt() !== null) {
          scheduler.runNext(); executed++;
          // Preserve per-callback errors and stop boundaries without building
          // public/private views, deployment cells and filtered state each tick.
          assertEngineHealthy();
          const ps = match.players.get(PLAYER);
          if (match.ended || !ps.alive || !!ps.left
              || match.phase !== phase || match.round !== round) break;
        }
      }
      if (executed) revision++;
      return stateReply({ executed });
    }
    case 'release': close(); summary = null; return { released: true };
    case 'close': close(); return { closed: true };
    default: throw new Error('Unknown bridge command');
  }
}
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
try {
  for await (const line of lines) {
    let req;
    try {
      req = JSON.parse(line);
      const response = dispatch(req);
      process.stdout.write(JSON.stringify({ ok: true, requestId: req.requestId, ...response }) + '\n');
      if (req.cmd === 'close') break;
    } catch (err) {
      // Fail closed: engine/transport exceptions cannot be used as rejected-action training samples.
      close();
      process.stdout.write(JSON.stringify({ ok: false, requestId: req?.requestId, error: err.message }) + '\n');
      break;
    }
  }
} finally { close(); }
