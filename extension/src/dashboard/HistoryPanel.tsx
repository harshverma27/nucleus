// History mode (M9): a per-version pass/fail bar chart over the ledger
// timeline. Pure display — the data comes pre-computed from
// `nucleus history --graph` (TrendData); this panel only draws it and offers a
// JSON export and a "last N" view filter.

import React, { useLayoutEffect, useMemo, useRef, useState } from "react";

import { TrendData, TrendPoint } from "./types";
import { download, setupCanvas } from "./util";

const PASS = "#43a047";
const FAIL = "#e53935";
const SKIP = "rgba(255,255,255,0.18)";

export function HistoryPanel({ trend }: { trend: TrendData }): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [limit, setLimit] = useState<number>(0); // 0 = all

  const points = useMemo(() => {
    const all = trend.points;
    return limit > 0 && limit < all.length ? all.slice(all.length - limit) : all;
  }, [trend, limit]);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const surface = setupCanvas(canvas);
    if (!surface) return;
    drawBars(surface.ctx, surface.width, surface.height, points);
  }, [points]);

  const totalPass = points.reduce((a, p) => a + p.pass, 0);
  const totalFail = points.reduce((a, p) => a + p.fail, 0);

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>History — pass / fail by version</h2>
        <span className="spacer" />
        <span className="history-legend">
          <span className="swatch" style={{ background: PASS }} /> pass
          <span className="swatch" style={{ background: FAIL }} /> fail
        </span>
        <select
          className="history-limit"
          value={limit}
          onChange={(e) => setLimit(Number(e.target.value))}
          title="Show last N versions"
        >
          <option value={0}>all ({trend.points.length})</option>
          <option value={10}>last 10</option>
          <option value={25}>last 25</option>
          <option value={50}>last 50</option>
        </select>
        <button
          onClick={() =>
            download("nucleus-trend.json", JSON.stringify(trend, null, 2))
          }
          title="Export the trend data as JSON"
        >
          ⬇ Export
        </button>
      </header>
      <div className="canvas-wrap">
        <canvas ref={canvasRef} />
        {points.length === 0 && (
          <div className="empty overlay">
            no recorded versions — run <code>nucleus build</code> then{" "}
            <code>nucleus test</code>
          </div>
        )}
      </div>
      <footer className="history-footer">
        {points.length} version{points.length === 1 ? "" : "s"} · {totalPass}{" "}
        passed · {totalFail} failed
      </footer>
    </section>
  );
}

function drawBars(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  points: TrendPoint[]
): void {
  ctx.clearRect(0, 0, width, height);
  if (points.length === 0) return;

  const padTop = 8;
  const padBottom = 22; // room for short-hash labels
  const plotH = Math.max(1, height - padTop - padBottom);

  // Y scale: tallest stacked (pass+fail+skip) bar, min 1.
  const maxTotal = Math.max(
    1,
    ...points.map((p) => p.pass + p.fail + p.skip)
  );

  // Baseline.
  ctx.strokeStyle = "rgba(255,255,255,0.12)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, padTop + plotH);
  ctx.lineTo(width, padTop + plotH);
  ctx.stroke();

  const n = points.length;
  const slot = width / n;
  const barW = Math.max(2, Math.min(28, slot * 0.6));

  ctx.font = "10px var(--vscode-font-family, monospace)";
  ctx.textAlign = "center";

  points.forEach((p, i) => {
    const cx = slot * (i + 0.5);
    const x = cx - barW / 2;
    const h = (v: number) => (v / maxTotal) * plotH;
    let y = padTop + plotH;

    // Stack fail (bottom), pass, then skip (top, muted).
    for (const [count, color] of [
      [p.fail, FAIL],
      [p.pass, PASS],
      [p.skip, SKIP],
    ] as const) {
      if (count <= 0) continue;
      const seg = h(count);
      y -= seg;
      ctx.fillStyle = color;
      ctx.fillRect(x, y, barW, seg);
    }

    // Mark unverified (conflicted) versions with a red outline.
    if (!p.approved) {
      ctx.strokeStyle = FAIL;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(x - 1, padTop + plotH - h(maxTotal), barW + 2, h(maxTotal));
    }

    // Short-hash label (only when slots are wide enough to avoid overlap).
    if (slot > 36) {
      ctx.fillStyle = "rgba(255,255,255,0.55)";
      ctx.fillText(p.short.slice(0, 7), cx, height - 8);
    }
  });
}
