"use strict";
const $ = (id) => document.getElementById(id);
const el = (tag, text, cls) => {
  const n = document.createElement(tag);
  if (text !== undefined) n.textContent = String(text);
  if (cls) n.className = cls;
  return n;
};
let catalog = { names: {}, stages: {} },
  files = [],
  selected = "",
  doc = null,
  index = 0,
  timer = null,
  busy = false,
  frameRequest = 0,
  documentRequest = 0;
const name = (p) => p?.name || catalog.names[p?.id] || p?.id || "空";
const fmt = (x) =>
  x === undefined || x === null
    ? "—"
    : typeof x === "number"
      ? Number.isInteger(x)
        ? String(x)
        : x.toFixed(4)
      : String(x);
const action = (a) =>
  a
    ? `${a.t || "动作"}${Object.entries(a)
        .filter(([k]) => k !== "t")
        .map(
          ([k, v]) => ` ${k}=${typeof v === "object" ? JSON.stringify(v) : v}`,
        )
        .join("")}`
    : "—";
function error(e) {
  $("error").hidden = !e;
  $("error").textContent = e?.message || "";
}
async function api(path, params = {}) {
  const r = await fetch(`${path}?${new URLSearchParams(params)}`);
  const d = await r.json();
  if (!r.ok) throw Error(d.error || r.statusText);
  return d;
}
function fileList() {
  const query = $("search").value.toLowerCase();
  const groups = new Map();
  for (const f of files) {
    if (!f.path.toLowerCase().includes(query)) continue;
    const group = f.path.includes("/") ? f.path.split("/")[0] : "根目录";
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(f);
  }
  const frag = document.createDocumentFragment();
  for (const [group, rows] of groups) {
    const details = el("details");
    details.open =
      !!query || rows.some((r) => r.path === selected) || groups.size <= 3;
    details.append(el("summary", `${group} · ${rows.length}`));
    for (const f of rows) {
      const b = el(
        "button",
        f.path.includes("/") ? f.path.slice(f.path.indexOf("/") + 1) : f.path,
        f.path === selected ? "active" : "",
      );
      b.append(
        el(
          "small",
          `${(f.size / 1024).toFixed(0)} KB · ${new Date(f.modified * 1000).toLocaleString()}`,
        ),
      );
      b.onclick = () => load(f.path);
      details.append(b);
    }
    frag.append(details);
  }
  $("files").replaceChildren(frag);
  if (!groups.size) $("files").append(el("p", "没有匹配文件", "muted"));
}
async function refresh() {
  const data = await api("/api/files");
  files = data.files;
  fileList();
  if (data.capped)
    error(Error("文件列表达到 10,000 条上限；请缩小 results-root"));
}
function stat(label, value, unit = "") {
  const n = el("div", undefined, "stat");
  n.append(el("label", label), el("strong", fmt(value)), el("small", unit));
  return n;
}
function metrics(raw) {
  const root = $("metrics");
  root.replaceChildren();
  const entries = [];
  function walk(x, p = "", depth = 0) {
    if (depth > 5 || entries.length >= 180) return;
    if (!x || typeof x !== "object") return;
    for (const [k, v] of Object.entries(x)) {
      if (Array.isArray(v)) continue;
      if (v && typeof v === "object") walk(v, p + k + ".", depth + 1);
      else if (typeof v === "number" || typeof v === "boolean")
        entries.push([p + k, v]);
    }
  }
  walk(raw);
  if (entries.length) {
    const table = el("table");
    for (const [k, v] of entries) {
      const tr = el("tr");
      tr.append(el("td", k), el("td", fmt(v)));
      table.append(tr);
    }
    root.append(table);
  }
}
function chart() {
  const rows = doc.timeline;
  $("charts").hidden = rows.length < 2;
  if (rows.length < 2) return;
  const width = 1000,
    height = 110;
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 1000 140");
  function series(key, color) {
    const valid = rows
      .map((r, i) => [i, r[key]])
      .filter(([, v]) => typeof v === "number");
    if (!valid.length) return;
    const max = Math.max(1, ...valid.map(([, v]) => v));
    const path = document.createElementNS(ns, "polyline");
    path.setAttribute(
      "points",
      valid
        .map(
          ([i, v]) =>
            `${10 + (i / (rows.length - 1)) * (width - 20)},${height - (v / max) * 90}`,
        )
        .join(" "),
    );
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", color);
    path.setAttribute("stroke-width", "2");
    svg.append(path);
  }
  series("hp", "#a4dfb6");
  series("funds", "#e4bc70");
  const labels = el("div", undefined, "chart-label");
  labels.append(
    el("span", "● 生命"),
    el("span", "● 资金"),
    el("span", "各自归一化 · 横轴为记录顺序"),
  );
  $("charts").replaceChildren(labels, svg);
}
async function load(path, following = false) {
  const request = ++documentRequest;
  try {
    if (!following) stop();
    const d = await api("/api/document", { path });
    if (request !== documentRequest) return;
    const atEnd = doc && index === doc.timeline.length - 1;
    doc = d;
    selected = path;
    if (!following) index = 0;
    else
      index = atEnd
        ? Math.max(0, doc.timeline.length - 1)
        : Math.min(index, Math.max(0, doc.timeline.length - 1));
    error(null);
    $("welcome").hidden = true;
    $("viewer").hidden = false;
    $("title").textContent = path.split("/").at(-1);
    $("path").textContent = path;
    $("status").textContent =
      typeof doc.status === "string" ? doc.status : JSON.stringify(doc.status);
    $("notice").textContent =
      `${doc.precision} · ${doc.format === "native" ? "状态快照浏览，不含逐帧战斗动画" : "仅展示文件实际记录的数据，不补造中间局面"}${doc.warning ? " · " + doc.warning : ""}${doc.format === "native" && !Object.keys(doc.footer).length ? " · 未闭合不等于正在训练" : ""}`;
    $("replay").hidden = !doc.timeline.length;
    $("raw-doc").textContent = JSON.stringify(
      doc.raw || { header: doc.header, footer: doc.footer },
      null,
      2,
    );
    metrics(doc.raw || {});
    training(doc.raw || {});
    chart();
    $("scrub").max = Math.max(0, doc.timeline.length - 1);
    const seen = new Set();
    $("round").replaceChildren();
    doc.timeline.forEach((r, i) => {
      if (!seen.has(r.round)) {
        seen.add(r.round);
        const o = el("option", `R${r.round ?? "?"}`);
        o.value = i;
        $("round").append(o);
      }
    });
    fileList();
    if (doc.timeline.length) await show(index);
    else {
      const raw = doc.raw || {};
      $("stats").replaceChildren(
        stat("文件类型", "指标"),
        stat("已完成", raw.completed ?? raw.games),
        stat("状态", doc.status),
      );
    }
    history.replaceState(null, "", `#${encodeURIComponent(path)}`);
  } catch (e) {
    if (request === documentRequest) error(e);
  }
}
function card(piece, slot) {
  const n = el(
    "button",
    piece ? name(piece) : "—",
    "card" + (piece?.golden ? " golden" : "") + (piece?.sold ? " sold" : ""),
  );
  n.append(
    el(
      "small",
      piece
        ? `#${slot}${piece.price !== undefined ? " · " + piece.price + " 资金" : ""}${piece.uid !== undefined ? " · UID " + piece.uid : ""}${piece.golden ? " · 金" : ""}`
        : `槽 ${slot}`,
    ),
  );
  n.onclick = () => detail(piece);
  return n;
}
function detail(p) {
  $("piece-detail").textContent = p
    ? `${name(p)} · UID ${fmt(p.uid)} · ${p.golden ? "金棋" : "普通"} · T${fmt(p.tier)} · 朝向 ${p.dir || "—"} · 装备 ${(p.items || []).map(name).join("、") || "无"}${p.ownerUid ? " · 归属 " + p.ownerUid : ""}`
    : "空槽";
}
function board(state) {
  const own = state.own || {};
  const pieces = own.board || [];
  const stage = catalog.stages[state.public?.stageId];
  const cells = own.deployment?.cells || [];
  const positions = [...cells, ...pieces].filter(
    (p) => Number.isInteger(p.row) && Number.isInteger(p.col),
  );
  let rows = stage?.rows;
  if (!Array.isArray(rows)) rows = [];
  let height = Math.max(
    rows.length,
    positions.length ? Math.max(...positions.map((p) => p.row)) + 2 : 14,
  );
  let width = Math.max(
    0,
    ...rows.map((r) => r.length),
    positions.length ? Math.max(...positions.map((p) => p.col)) + 2 : 12,
  );
  height = Math.min(40, height);
  width = Math.min(40, width);
  const container = $("board");
  container.style.gridTemplateColumns = `repeat(${width}, minmax(0, 1fr))`;
  const deployed = new Map(cells.map((c) => [`${c.row},${c.col}`, c]));
  const units = new Map(pieces.map((p) => [`${p.row},${p.col}`, p]));
  const frag = document.createDocumentFragment();
  const arrows = { UP: "↑", DOWN: "↓", LEFT: "←", RIGHT: "→" };
  for (let r = 0; r < height; r++)
    for (let c = 0; c < width; c++) {
      const tile = stage?.tiles?.[rows[r]?.[c]];
      const type =
        tile?.buildable === "ALL" ||
        tile?.buildable === "MELEE" ||
        tile?.buildable === "RANGED" ||
        deployed.has(`${r},${c}`)
          ? "tile"
          : tile?.groundPassable
            ? "road"
            : "blocked";
      const cell = el("div", undefined, `cell ${type}`);
      cell.title = `行 ${r} · 列 ${c}${tile ? " · " + tile.tileKey : ""}`;
      const p = units.get(`${r},${c}`);
      if (p) {
        const b = el(
          "button",
          undefined,
          "unit" +
            (p.golden ? " golden" : "") +
            (p.kind === "token" ? " token" : ""),
        );
        b.append(
          el("span", name(p)),
          el("small", `#${p.uid} ${arrows[p.dir] || ""}`),
        );
        b.title = JSON.stringify(p);
        b.onclick = () => detail(p);
        cell.append(b);
      } else if (deployed.has(`${r},${c}`))
        cell.append(el("span", `${r},${c}`, "coord"));
      frag.append(cell);
    }
  container.replaceChildren(frag);
  $("board-meta").textContent =
    `${pieces.length} 个单位 · ${stage?.name || "坐标示意图"}`;
  $("hand").replaceChildren(...(own.hand || []).map(card));
  if (own.temp?.some(Boolean))
    $("hand").append(el("span", "临时区", "muted"), ...own.temp.map(card));
  $("shop").replaceChildren(...(own.shop?.slots || []).map(card));
  if (!own.shop) $("shop").append(el("span", "此快照未记录商店", "muted"));
  $("shop-meta").textContent = own.shop
    ? `等级 ${own.shop.level} · 刷新 ${own.shop.refreshPrice} · 升本 ${own.shop.upgradePrice}`
    : "";
  $("bonds").replaceChildren(
    ...(own.bonds || [])
      .filter((b) => b.active)
      .map((b) =>
        el(
          "span",
          `${catalog.names[b.bondId] || b.bondId} · ${b.count} 人 · ${b.layers ?? 0} 层`,
          "bond",
        ),
      ),
  );
}
function events(frame) {
  const source = frame.events || [frame.record];
  const root = $("events");
  root.replaceChildren();
  $("event-count").textContent = `${source.length} 条`;
  for (const e of source) {
    const n = el(
      "div",
      undefined,
      "event" +
        (e.accepted === false ? " bad" : "") +
        (e.forced ? " forced" : ""),
    );
    n.append(
      el("div", e.action ? action(e.action) : e.kind || e.status || "状态快照"),
    );
    const meta = [];
    if (e.round !== undefined) meta.push(`R${e.round}`);
    if (e.accepted !== undefined) meta.push(e.accepted ? "已接受" : "拒绝");
    if (e.funds !== undefined)
      meta.push(
        `资金 ${e.funds}${e.funds_after !== undefined ? " → " + e.funds_after : ""}`,
      );
    if (e.forced) meta.push(`干预：${e.forced}`);
    if (e.status) meta.push(e.status);
    if (e.hp_loss !== undefined) meta.push(`本战掉血 ${e.hp_loss}`);
    n.append(el("div", meta.join(" · "), "meta"));
    root.append(n);
  }
  if (frame.record?.menus) {
    const record = frame.record;
    const table = el("table");
    const labels = el("tr");
    labels.append(
      el("th", "模型"),
      el("th", "原动作概率"),
      el("th", "变化后动作概率"),
    );
    table.append(labels);
    for (const [model, menu] of Object.entries(record.menus)) {
      if (!menu || typeof menu !== "object") continue;
      const row = el("tr");
      const percent = (value) =>
        typeof value === "number" ? (value * 100).toFixed(3) + "%" : "—";
      row.append(
        el("td", model),
        el("td", percent(menu.control_probability)),
        el("td", percent(menu.endpoint_probability)),
      );
      table.append(row);
    }
    for (const [label, choice] of [
      ["原动作", record.control_choice],
      ["变化后", record.endpoint_choice],
    ]) {
      if (choice)
        root.append(
          el(
            "p",
            `${label}：${action(choice.action)}${choice.object ? " · " + name(choice.object) : ""}`,
          ),
        );
    }
    root.append(table);
    const d = el("details");
    d.append(
      el("summary", "策略候选 / 概率"),
      el("pre", JSON.stringify(frame.record.menus, null, 2)),
    );
    root.append(d);
  }
}
async function show(i) {
  if (!doc?.timeline.length) return;
  index = Math.max(0, Math.min(doc.timeline.length - 1, Number(i)));
  const request = ++frameRequest;
  const path = selected;
  try {
    const frame = await api("/api/frame", { path, index });
    if (request !== frameRequest || path !== selected) return;
    const state = frame.state || {},
      own = state.own || {},
      raw = doc.raw || {};
    $("scrub").value = index;
    $("position").textContent = `${index + 1} / ${doc.timeline.length}`;
    $("prev").disabled = index === 0;
    $("next").disabled = index === doc.timeline.length - 1;
    $("phase").textContent = `R${state.round ?? "?"} · ${state.phase || ""}`;
    const options = [...$("round").options];
    for (const o of options)
      if (Number(o.value) <= index) $("round").value = o.value;
    $("stats").replaceChildren(
      stat("当前回合", state.round, "ROUND"),
      stat("生命", own.lp ?? own.hp, "HP"),
      stat("剩余资金", own.funds),
      stat("通过轮数", raw.rounds_passed ?? raw.control_rounds),
      stat("整局回报", raw.episode_return),
    );
    board(state);
    events(frame);
    $("raw-frame").textContent = JSON.stringify(frame.record, null, 2);
    error(null);
  } catch (e) {
    error(e);
    stop();
  }
}
function stop() {
  clearInterval(timer);
  timer = null;
  $("play").textContent = "播放";
}
$("refresh").onclick = () => refresh().catch(error);
$("search").oninput = fileList;
$("scrub").oninput = () => {
  stop();
  show($("scrub").value);
};
$("prev").onclick = () => {
  stop();
  show(index - 1);
};
$("next").onclick = () => {
  stop();
  show(index + 1);
};
$("round").onchange = () => {
  stop();
  show($("round").value);
};
$("play").onclick = () => {
  if (timer) {
    stop();
    return;
  }
  if (index >= doc.timeline.length - 1) show(0);
  $("play").textContent = "暂停";
  timer = setInterval(() => {
    if (index >= doc.timeline.length - 1) stop();
    else show(index + 1);
  }, 650);
};
setInterval(async () => {
  if (!$("follow").checked || busy || timer) return;
  busy = true;
  try {
    await refresh();
    if (selected) await load(selected, true);
  } catch (e) {
    error(e);
  } finally {
    busy = false;
  }
}, 3000);
(async () => {
  try {
    catalog = await api("/api/catalog");
    await refresh();
    if (location.hash.length > 1)
      await load(decodeURIComponent(location.hash.slice(1)));
  } catch (e) {
    error(e);
  }
})();

window.addEventListener("hashchange", () => {
  const path = decodeURIComponent(location.hash.slice(1));
  if (path && path !== selected) load(path);
});
function training(raw) {
  const series = [];
  function visit(value, path = "", depth = 0) {
    if (depth > 7 || series.length >= 60 || !value || typeof value !== "object")
      return;
    for (const [key, child] of Object.entries(value)) {
      const label = path ? path + "." + key : key;
      if (Array.isArray(child)) {
        if (child.length > 1 && child.every((v) => typeof v === "number")) {
          if (/(returns|rounds|loss|kl|grad|reward|accuracy)/i.test(key))
            series.push({ label, values: child.slice(0, 10000) });
        } else if (
          ["history", "iterations", "train", "training"].includes(key) &&
          child.length > 1 &&
          child.every((v) => v && typeof v === "object" && !Array.isArray(v))
        ) {
          const keys = new Set(
            child.flatMap((v) =>
              Object.keys(v).filter((k) => typeof v[k] === "number"),
            ),
          );
          for (const k of keys) {
            if (series.length >= 60) break;
            series.push({
              label: label + "." + k,
              values: child
                .slice(0, 10000)
                .map((v) => (typeof v[k] === "number" ? v[k] : null)),
            });
          }
        } else if (child.length <= 20) {
          child.forEach((v, i) => visit(v, label + "[" + i + "]", depth + 1));
        }
      } else if (typeof child === "object") visit(child, label, depth + 1);
    }
  }
  visit(raw);
  $("training").hidden = !series.length;
  $("metric-select").replaceChildren();
  series.forEach((s, i) => {
    const o = el("option", s.label);
    o.value = i;
    $("metric-select").append(o);
  });
  function draw() {
    const s = series[Number($("metric-select").value)];
    if (!s) return;
    const valid = s.values
      .map((v, i) => [i, v])
      .filter(([, v]) => typeof v === "number" && Number.isFinite(v));
    if (!valid.length) return;
    const min = Math.min(...valid.map(([, v]) => v)),
      max = Math.max(...valid.map(([, v]) => v));
    const ns = "http://www.w3.org/2000/svg",
      svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 1000 160");
    const line = document.createElementNS(ns, "polyline");
    line.setAttribute(
      "points",
      valid
        .map(
          ([i, v]) =>
            `${15 + (i / Math.max(1, s.values.length - 1)) * 970},${130 - ((v - min) / (max - min || 1)) * 110}`,
        )
        .join(" "),
    );
    line.setAttribute("fill", "none");
    line.setAttribute("stroke", "#a4dfb6");
    line.setAttribute("stroke-width", "2");
    svg.append(line);
    $("metric-chart").replaceChildren(
      svg,
      el(
        "div",
        `min ${fmt(min)} · max ${fmt(max)} · ${s.values.length} 条 · 横轴为记录顺序`,
        "muted",
      ),
    );
  }
  $("metric-select").onchange = draw;
  draw();
}
