// The variable timeline: a live Canvas line chart of every traced variable
// (ports 1–7), with an auto-scaling Y axis and a rolling time window on X.

import React, { useLayoutEffect, useRef } from "react";

import { Series } from "./types";
import { TraceStore } from "./store";
import { setupCanvas } from "./util";

/** Visible time window, in milliseconds. */
const WINDOW_MS = 30_000;

export function VariableChart({
  store,
  tick,
}: {
  store: TraceStore;
  tick: number;
}): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const surface = setupCanvas(canvas);
    if (!surface) return;
    drawChart(surface.ctx, surface.width, surface.height, [...store.series.values()]);
    // `tick` drives the redraw; series data is read fresh each frame.
  }, [tick, store]);

  const series = [...store.series.values()];

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Variables</h2>
        <div className="legend">
          {series.map((s) => (
            <span className="legend-item" key={s.name} style={{ color: s.color }}>
              ● {s.name}
              <span className="legend-value">{lastValue(s)}</span>
            </span>
          ))}
        </div>
      </header>
      <div className="canvas-wrap">
        <canvas ref={canvasRef} />
        {series.length === 0 && <div className="empty overlay">waiting for variables…</div>}
      </div>
    </section>
  );
}

function lastValue(s: Series): string {
  const p = s.points[s.points.length - 1];
  if (!p) return "—";
  return s.type === "f32" ? p.v.toFixed(3) : String(p.v);
}

function drawChart(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  series: Series[]
): void {
  ctx.clearRect(0, 0, width, height);

  const pad = { left: 48, right: 8, top: 8, bottom: 18 };
  const plotW = Math.max(1, width - pad.left - pad.right);
  const plotH = Math.max(1, height - pad.top - pad.bottom);

  const now = Date.now();
  const tMin = now - WINDOW_MS;

  // Auto-scale Y across all points currently in the window.
  let yMin = Infinity;
  let yMax = -Infinity;
  for (const s of series) {
    for (const p of s.points) {
      if (p.t < tMin) continue;
      if (p.v < yMin) yMin = p.v;
      if (p.v > yMax) yMax = p.v;
    }
  }
  if (!isFinite(yMin) || !isFinite(yMax)) {
    yMin = 0;
    yMax = 1;
  }
  if (yMin === yMax) {
    yMin -= 1;
    yMax += 1;
  }
  const pdg = (yMax - yMin) * 0.1;
  yMin -= pdg;
  yMax += pdg;

  const xOf = (t: number) => pad.left + ((t - tMin) / WINDOW_MS) * plotW;
  const yOf = (v: number) => pad.top + (1 - (v - yMin) / (yMax - yMin)) * plotH;

  // Grid + Y labels.
  ctx.strokeStyle = "rgba(255,255,255,0.08)";
  ctx.fillStyle = "rgba(255,255,255,0.45)";
  ctx.font = "10px ui-monospace, monospace";
  ctx.lineWidth = 1;
  const rows = 4;
  for (let i = 0; i <= rows; i++) {
    const y = pad.top + (i / rows) * plotH;
    const v = yMax - (i / rows) * (yMax - yMin);
    ctx.beginPath();
    ctx.moveTo(pad.left, y);
    ctx.lineTo(width - pad.right, y);
    ctx.stroke();
    ctx.fillText(formatTick(v), 4, y + 3);
  }

  // Series lines.
  ctx.lineWidth = 1.5;
  for (const s of series) {
    ctx.strokeStyle = s.color;
    ctx.beginPath();
    let started = false;
    for (const p of s.points) {
      if (p.t < tMin) continue;
      const x = xOf(p.t);
      const y = yOf(p.v);
      if (started) {
        ctx.lineTo(x, y);
      } else {
        ctx.moveTo(x, y);
        started = true;
      }
    }
    ctx.stroke();
  }
}

function formatTick(v: number): string {
  if (Math.abs(v) >= 1000 || (v !== 0 && Math.abs(v) < 0.01)) {
    return v.toExponential(1);
  }
  return v.toFixed(2);
}
