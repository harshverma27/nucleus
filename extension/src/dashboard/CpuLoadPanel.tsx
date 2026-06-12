// The CPU-load strip chart: a rolling filled area of utilization (0–100%)
// derived from DWT PC-sampling packets by the daemon.

import React, { useLayoutEffect, useRef } from "react";

import { TraceStore } from "./store";
import { setupCanvas } from "./util";

export function CpuLoadPanel({
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
    drawStrip(surface.ctx, surface.width, surface.height, store.cpu.map((p) => p.v));
  }, [tick, store]);

  const current = store.cpu[store.cpu.length - 1];
  const pct = current ? Math.round(current.v * 100) : null;

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>CPU load</h2>
        <span className="cpu-value">{pct === null ? "—" : `${pct}%`}</span>
      </header>
      <div className="canvas-wrap">
        <canvas ref={canvasRef} />
        {store.cpu.length === 0 && (
          <div className="empty overlay">no DWT PC samples</div>
        )}
      </div>
    </section>
  );
}

function drawStrip(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  values: number[]
): void {
  ctx.clearRect(0, 0, width, height);

  // Horizontal guides at 0 / 50 / 100%.
  ctx.strokeStyle = "rgba(255,255,255,0.08)";
  ctx.lineWidth = 1;
  for (const frac of [0, 0.5, 1]) {
    const y = height - frac * height;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();
  }

  if (values.length === 0) return;

  const n = values.length;
  const xOf = (i: number) => (n === 1 ? width : (i / (n - 1)) * width);
  const yOf = (v: number) => height - Math.max(0, Math.min(1, v)) * height;

  ctx.beginPath();
  ctx.moveTo(0, height);
  values.forEach((v, i) => ctx.lineTo(xOf(i), yOf(v)));
  ctx.lineTo(xOf(n - 1), height);
  ctx.closePath();
  ctx.fillStyle = "rgba(79,195,247,0.25)";
  ctx.fill();

  ctx.beginPath();
  values.forEach((v, i) => (i === 0 ? ctx.moveTo(xOf(i), yOf(v)) : ctx.lineTo(xOf(i), yOf(v))));
  ctx.strokeStyle = "#4fc3f7";
  ctx.lineWidth = 1.5;
  ctx.stroke();
}
