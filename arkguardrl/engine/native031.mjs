// Independent native-intent bridge for the frozen Stronghold Protocol 0.3.1 engine.
// Engine implementation stays external; no legacy ArkGuardRL adapter is imported.
import fs from 'node:fs';
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
function observe() {
  if (!match) throw new Error('Reset required');
  if (match.errorCount || scheduler.errors.length || match.simErrors || match.dispatcher?.errors) throw new Error('Engine failure');
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
        return { state: observe() };
      } catch (err) { close(); throw err; }
    }
    case 'observe': return { state: observe() };
    case 'act': {
      observe();
      if (!req.action || !ALLOWED.has(req.action.t)) throw new Error('Unsupported native player intent');
      const error = protocol.validateC2S(req.action);
      if (error) throw new Error(`Invalid intent: ${error}`);
      const result = match.handle(PLAYER, req.action);
      revision++;
      return { accepted: !!result?.ok, result: visible(result), state: observe() };
    }
    case 'advance': {
      const before = observe();
      const steps = req.maxSteps ?? 256;
      if (!Number.isInteger(steps) || steps < 1 || steps > 4096) throw new Error('maxSteps must be 1..4096');
      // Explicit clock progression only outside human decision phases. No invented solo deadlines.
      let executed = 0;
      if (!before.gameEnded && !before.public.paused && !playerDecisionPending()) {
        while (executed < steps && scheduler.nextAt() !== null) {
          scheduler.runNext(); executed++;
          const now = observe();
          if (now.personalEnded || now.phase !== before.phase || now.round !== before.round) break;
        }
      }
      if (executed) revision++;
      return { executed, state: observe() };
    }
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
