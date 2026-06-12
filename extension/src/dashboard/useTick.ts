// A render clock. `useTick(fps)` forces the calling component to re-render at up
// to `fps` frames per second, so views that read mutable store data (logs,
// charts) refresh smoothly without the store needing fine-grained subscriptions.

import { useEffect, useState } from "react";

export function useTick(fps: number): number {
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let raf = 0;
    let last = 0;
    const interval = 1000 / fps;
    const loop = (ts: number) => {
      if (ts - last >= interval) {
        last = ts;
        setTick((t) => t + 1);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [fps]);

  return tick;
}
