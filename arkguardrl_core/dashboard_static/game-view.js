// Adapter only. Renderer, geometry, picking and art loaders come from the external game.
let initialization = null,
  view = null,
  latest = null,
  selectPiece = null;
let data = null,
  geo = null,
  engine = null,
  generation = 0,
  activeStage = null,
  controller = null,
  documentPath = null;
const $ = (id) => document.getElementById(id);
const status = (text) => {
  $("renderer-status").textContent = text;
};

async function json(path) {
  const response = await fetch(path);
  if (!response.ok) throw Error(`${path}: ${response.status}`);
  return response.json();
}
async function initialize(signal) {
  engine = (await json("/api/catalog")).renderer;
  if (!engine?.enabled) throw Error("启动 dashboard 时需要 --engine-root");
  if (!engine.vendor)
    throw Error("game 缺少 public/vendor；可用 --vendor-root 指向已有依赖目录");
  const names = [
    "chess",
    "tokens",
    "items",
    "enemies",
    "stages",
    "bonds",
    "config",
    "backups",
  ];
  const [renderer, assetsModule, constants, entries] = await Promise.all([
    import("/js/render/app.js"),
    import("/js/assets.js"),
    import("/shared/constants.js"),
    Promise.all(
      names.map(async (name) => [name, await json(`/data/${name}.json`)]),
    ),
  ]);
  signal.throwIfAborted();
  const loaded = Object.fromEntries(entries);
  const field = await renderer.createFieldView($("game-field"), {
    data: loaded,
    signal,
    assets: assetsModule.createAssets(),
    board: "3d",
    settings: { quality: "medium", officialFx: "off", damageNumbers: false },
  });
  signal.throwIfAborted();
  view = field;
  data = loaded;
  geo = constants.GEO;
  const selector = $("game-stage");
  for (const [id, stage] of Object.entries(data.stages)) {
    const option = document.createElement("option");
    option.value = id;
    option.textContent = `${stage.name || id} · ${id}`;
    selector.append(option);
  }
  view.on("pieceClick", (event) => {
    if (event.piece) selectPiece?.(event.piece);
    else if (event.uid != null) {
      const own = latest?.own || {};
      const piece = [
        ...(own.board || []),
        ...(own.hand || []),
        ...(own.temp || []),
      ].find((p) => p?.uid === event.uid);
      if (piece) selectPiece?.(piece);
    }
  });
  selector.onchange = () => apply();
  $("game-reset").onclick = () => {
    if (activeStage) frameCamera();
  };
  $("game-mode").onclick = async () => {
    try {
      const on = view.stats().board3d?.on;
      await view.setBoardMode(on ? "2d" : "3d");
      reportStatus();
    } catch (error) {
      status("game 切换视图失败：" + error.message);
    }
  };
}
function frameCamera() {
  // Use the same framing as game/dev/render-demo.js, not a dashboard camera implementation.
  const field = latest?.own?.deployment?.field;
  if (field === "boss" || field === "hidden")
    view.setCamera("bossPrep", { side: "L", instant: true });
  else
    view.setCamera("prep", {
      rect: { ...geo.NORMAL_RECT },
      side: "L",
      shop: false,
      instant: true,
    });
}
function reportStatus() {
  const threeD = view.stats().board3d?.on;
  $("game-mode").textContent = threeD ? "切到原生 2D" : "请求原生 3D";
  status(
    `game createFieldView · ${threeD ? "原生 3D" : "原生 2D"} · 只读状态快照${!engine.art ? " · 本机未安装棋盘贴图和角色素材，使用 game 自带占位显示" : ""}${!latest?.public?.stageId ? " · 地图由手动选择，未写回轨迹" : ""}`,
  );
}
function apply() {
  if (!view || !latest) return;
  const recorded = latest.public?.stageId;
  const selector = $("game-stage");
  selector.disabled = !!recorded;
  if (recorded) selector.value = recorded;
  const stageId = recorded || selector.value;
  const stage = data.stages[stageId];
  if (!stage) {
    activeStage = null;
    $("game-field").hidden = true;
    status("该快照没有地图标识；请先选择实际地图。");
    return;
  }
  $("game-field").hidden = false;
  activeStage = stageId;
  view.setStage(stage);
  const own = latest.own || {};
  view.setPrep(
    {
      ...own,
      hand: own.hand || Array(10).fill(null),
      temp: own.temp || Array(5).fill(null),
      board: own.board || [],
    },
    { editable: false },
  );
  frameCamera();
  view.resize();
  reportStatus();
}
export async function renderState(state, onSelect, path) {
  if (path !== documentPath) {
    $("game-stage").value = "";
    documentPath = path;
  }
  latest = state;
  selectPiece = onSelect;
  const current = ++generation;
  if (!initialization) {
    status("正在加载 game 原生渲染器…");
    controller = new AbortController();
    const loading = controller;
    initialization = initialize(loading.signal).catch((error) => {
      if (controller === loading) initialization = null;
      throw error;
    });
  }
  try {
    await initialization;
  } catch (error) {
    if (current === generation) throw error;
    else return;
  }
  if (current === generation) apply();
}
export function clear() {
  generation++;
  controller?.abort();
  controller = null;
  latest = null;
  activeStage = null;
  documentPath = null;
  view?.destroy();
  view = null;
  initialization = null;
  data = null;
  $("game-stage").replaceChildren(new Option("记录未包含地图，请选择", ""));
}
window.addEventListener("pagehide", () => view?.destroy());
