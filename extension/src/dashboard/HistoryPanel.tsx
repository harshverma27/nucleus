// History mode: a per-run pass/fail bar chart over the test-history timeline.
// Pure display — the counts come pre-computed from `nucleus history --graph`
// (HistorySummary); this panel only draws them, plus a JSON export and a
// "last N" view filter.

import React, { useLayoutEffect, useMemo, useRef, useState } from "react";

import { HistorySummary, RunSummary } from "./types";
import { download, setupCanvas } from "./util";

const PASS = "#43a047";
const FAIL = "#e53935";
const SKIP = "rgba(255,255,255,0.18)";

export function HistoryPanel({
  summary,
}: {
  summary: HistorySummary;
}): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [limit, setLimit] = useState<number>(0); // 0 = all

  const runs = useMemo(() => {
    const all = summary.runs;
    return limit > 0 && limit < all.length ? all.slice(all.length - limit) : all;
  }, [summary, limit]);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const surface = setupCanvas(canvas);
    if (!surface) return;
    drawBars(surface.ctx, surface.width, surface.height, runs);
  }, [runs]);

  const totalPass = runs.reduce((a, r) => a + r.pass, 0);
  const totalFail = runs.reduce((a, r) => a + r.fail, 0);

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>History — pass / fail by run</h2>
        <span className="spacer" />
        <span className="history-legend">
          <span className="swatch" style={{ background: PASS }} /> pass
          <span className="swatch" style={{ background: FAIL }} /> fail
        </span>
        <select
          className="history-limit"
          value={limit}
          onChange={(e) => setLimit(Number(e.target.value))}
          title="Show last N runs"
        >
          <option value={0}>all ({summary.runs.length})</option>
          <option value={10}>last 10</option>
          <option value={25}>last 25</option>
          <option value={50}>last 50</option>
        </select>
        <button
          onClick={() =>
            download("test_history_summary.json", JSON.stringify(summary, null, 2))
          }
          title="Export the run summary as JSON"
        >
          ⬇ Export
        </button>
      </header>
      <div className="canvas-wrap">
        <canvas ref={canvasRef} />
        {runs.length === 0 && (
          <div className="empty overlay">
            no recorded runs — run <code>nucleus test</code>
          </div>
        )}
      </div>
      <footer className="history-footer">
        {runs.length} run{runs.length === 1 ? "" : "s"} · {totalPass} passed ·{" "}
        {totalFail} failed
      </footer>
    </section>
  );
}

function drawBars(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  runs: RunSummary[]
): void {
  ctx.clearRect(0, 0, width, height);
  if (runs.length === 0) return;

  const padTop = 8;
  const padBottom = 22; // room for run-number labels
  const plotH = Math.max(1, height - padTop - padBottom);

  // Y scale: tallest stacked (pass+fail+skip) bar, min 1.
  const maxTotal = Math.max(1, ...runs.map((r) => r.pass + r.fail + r.skip));

  // Baseline.
  ctx.strokeStyle = "rgba(255,255,255,0.12)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, padTop + plotH);
  ctx.lineTo(width, padTop + plotH);
  ctx.stroke();

  const n = runs.length;
  const slot = width / n;
  const barW = Math.max(2, Math.min(28, slot * 0.6));

  ctx.font = "10px var(--vscode-font-family, monospace)";
  ctx.textAlign = "center";

  runs.forEach((r, i) => {
    const cx = slot * (i + 0.5);
    const x = cx - barW / 2;
    const h = (v: number) => (v / maxTotal) * plotH;
    let y = padTop + plotH;

    // Stack fail (bottom), pass, then skip (top, muted).
    for (const [count, color] of [
      [r.fail, FAIL],
      [r.pass, PASS],
      [r.skip, SKIP],
    ] as const) {
      if (count <= 0) continue;
      const seg = h(count);
      y -= seg;
      ctx.fillStyle = color;
      ctx.fillRect(x, y, barW, seg);
    }

    // Run-number label (only when slots are wide enough to avoid overlap).
    if (slot > 26) {
      ctx.fillStyle = "rgba(255,255,255,0.55)";
      ctx.fillText(`#${i + 1}`, cx, height - 8);
    }
  });
}
