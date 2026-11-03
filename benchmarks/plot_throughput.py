"""Render the recorded throughput results; does not import or run the environment."""
import json
from pathlib import Path
import xml.etree.ElementTree as ET

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D


root = Path(__file__).resolve().parents[1]
source = root / "benchmarks/env-hotspots-20261102.json"
output = root / "docs/assets/benchmark-throughput.svg"
data = json.loads(source.read_text())
plt.rcParams.update({"font.family": "DejaVu Sans", "svg.fonttype": "path",
                     "svg.hashsalt": "arkguardrl-throughput", "font.size": 10})
fig, ax = plt.subplots(figsize=(11, 5.4), facecolor="white")
fig.subplots_adjust(left=.16, right=.82, bottom=.24, top=.73)
fig.text(.055, .91, "Environment throughput", fontsize=21, weight="bold", color="#0f172a")
fig.text(.055, .85, "0.3.1 engine (Node.js) + Rust action / observation kernels  |  2026-11-02",
         fontsize=10.5, color="#475569")
colors = {"before": "#64748b", "after": "#0f766e"}
fills = {"before": "#e2e8f0", "after": "#ccfbf1"}
fig.legend(handles=[Line2D([], [], color=colors[key], lw=7, label=label)
                    for key, label in (("before", "Before"), ("after", "After"))],
           loc="upper left", bbox_to_anchor=(.05, .81), frameon=False, ncol=2, fontsize=10)
fig.text(.84, .765, "MEDIAN", fontsize=9, color="#64748b")
jitter = [-.06, .06, -.10, .10, -.03, .03]
for config, title, top in (("sync-1", "1 environment · Sync", 3.25),
                            ("async-4", "4 environments · Async / shared memory", 1.35)):
    item = data["measurements"][config]
    ax.text(-.15, top + .48, title, transform=ax.get_yaxis_transform(),
            color="#0f172a", weight="bold", fontsize=10.5)
    for variant, offset in (("before", 0), ("after", -.65)):
        y = top + offset
        values, median = item[f"{variant}_rates"], item["medians"][variant]
        ax.barh(y, median, height=.24, color=fills[variant], zorder=2)
        ax.scatter(values, [y + j for j in jitter], color=colors[variant],
                   s=31, edgecolor="white", linewidth=.6, zorder=4)
        ax.vlines(median, y - .20, y + .20, color=colors[variant], linewidth=2.4, zorder=5)
        ax.text(-.025, y, variant.title(), transform=ax.get_yaxis_transform(),
                ha="right", va="center", color="#475569", fontsize=9.5)
        ax.text(1.035, y, f"{median:,.2f}", transform=ax.get_yaxis_transform(),
                va="center", color=colors[variant], fontsize=12, weight="bold")
ax.set(xlim=(0, 1050), ylim=(.15, 3.95), yticks=[], xticks=[0, 250, 500, 750, 1000])
ax.set_xlabel("Total environment decisions per second", color="#475569", labelpad=10)
ax.grid(axis="x", color="#e2e8f0", linewidth=.8, zorder=0)
ax.tick_params(axis="x", length=0, pad=7, colors="#64748b")
for spine in ax.spines.values():
    spine.set_visible(False)
fig.text(.055, .125, "Dots: all 6 runs per version. Vertical ticks: medians.  |  Apple M5 / 16 GiB",
         fontsize=9.5, color="#475569")
fig.text(.055, .075, "1,024 decisions/env · 64 warm-up steps · loop resets included · no model or training",
         fontsize=9, color="#64748b")
fig.text(.055, .03, "Shared-desktop timings vary; small gains do not establish a stable training speedup.",
         fontsize=9, color="#64748b")
output.parent.mkdir(parents=True, exist_ok=True)
fig.savefig(output, format="svg", metadata={"Date": None, "Creator": "ArkGuardRL-Core"})
plt.close(fig)

ns = "http://www.w3.org/2000/svg"
ET.register_namespace("", ns)
ET.register_namespace("xlink", "http://www.w3.org/1999/xlink")
tree = ET.parse(output)
svg = tree.getroot()
svg.set("role", "img")
svg.set("aria-labelledby", "benchmark-title benchmark-desc")
title = ET.Element(f"{{{ns}}}title", id="benchmark-title")
title.text = "Environment throughput on 2026-11-02: six runs and medians"
desc = ET.Element(f"{{{ns}}}desc", id="benchmark-desc")
desc.text = ("Apple M5, 16 GiB. Engine 0.3.1 on Node.js with Rust action/observation kernels. "
             "Uniform candidate sampling, no model or training. The zero-based horizontal axis "
             "shows total decisions per second; dots show all six repeated timings per version. "
             "Single environment median: 480.57 before, 494.75 after. Four asynchronous environments "
             "with shared memory: 899.14 before, 937.19 after. Each run measures 1024 decisions "
             "per environment after 64 warm-up steps. Loop resets count; initial reset and setup "
             "do not. Shared-desktop variation prevents claiming a stable training speedup. "
             "Source: benchmarks/env-hotspots-20261102.json.")
svg.insert(0, title)
svg.insert(1, desc)
tree.write(output, encoding="utf-8", xml_declaration=True)
print(output)
