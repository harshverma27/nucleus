// The dashboard's in-memory trace store.
//
// A single `TraceStore` owns all received data and the WebSocket connection. It
// is deliberately plain (not React state): high-frequency trace data is mutated
// in place here, and the UI re-reads it on an animation-frame tick (see
// `useTick`). This keeps sub-millisecond data rates from drowning React in
// re-renders. Buffers are capped so memory stays bounded during long sessions.

import { ConnectionStatus, LogEntry, Point, Series, TraceEvent } from "./types";

const MAX_LOGS = 5000;
const MAX_POINTS = 2000;
const MAX_CPU = 600;

// Up to 7 simultaneous variables (ports 1–7), each a distinct hue.
const COLORS = [
  "#4fc3f7",
  "#aed581",
  "#ff8a65",
  "#ba68c8",
  "#ffd54f",
  "#4db6ac",
  "#f06292",
];

export class TraceStore {
  logs: LogEntry[] = [];
  series = new Map<string, Series>();
  cpu: Point[] = [];
  overflow = 0;
  status: ConnectionStatus = "connecting";

  private ws?: WebSocket;
  private nextId = 0;

  constructor(private readonly url: string) {}

  /** Open the WebSocket (with auto-reconnect). Returns a disposer. */
  connect(): () => void {
    let disposed = false;

    const open = () => {
      this.status = "connecting";
      const ws = new WebSocket(this.url);
      this.ws = ws;
      ws.onopen = () => {
        this.status = "open";
      };
      ws.onmessage = (e) => {
        try {
          this.handle(JSON.parse(e.data as string) as TraceEvent);
        } catch {
          // Ignore malformed frames; the daemon only emits valid JSON.
        }
      };
      ws.onclose = () => {
        this.status = "closed";
        if (!disposed) {
          setTimeout(open, 1000);
        }
      };
      ws.onerror = () => ws.close();
    };

    open();
    return () => {
      disposed = true;
      this.ws?.close();
    };
  }

  /** Drop all accumulated data (the "clear" button). */
  clear(): void {
    this.logs = [];
    this.series.clear();
    this.cpu = [];
    this.overflow = 0;
  }

  private handle(ev: TraceEvent): void {
    const now = Date.now();
    switch (ev.kind) {
      case "log":
        this.logs.push({ id: this.nextId++, t: now, message: ev.message });
        trimFront(this.logs, MAX_LOGS);
        break;
      case "variable": {
        let s = this.series.get(ev.name);
        if (!s) {
          s = {
            name: ev.name,
            port: ev.port,
            type: ev.type,
            color: COLORS[this.series.size % COLORS.length],
            points: [],
          };
          this.series.set(ev.name, s);
        }
        s.points.push({ t: now, v: ev.value });
        trimFront(s.points, MAX_POINTS);
        break;
      }
      case "cpuload":
        this.cpu.push({ t: now, v: ev.load });
        trimFront(this.cpu, MAX_CPU);
        break;
      case "overflow":
        this.overflow++;
        break;
    }
  }
}

function trimFront<T>(arr: T[], max: number): void {
  if (arr.length > max) {
    arr.splice(0, arr.length - max);
  }
}
