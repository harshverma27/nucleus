// A two-child resizable split with a draggable divider. Composing two of these
// gives the dashboard's three independently resizable panels.

import React, { useCallback, useRef, useState } from "react";

interface SplitPaneProps {
  direction: "horizontal" | "vertical";
  /** Initial size of the first child, as a percentage (0–100). */
  initial: number;
  /** Minimum size of either child, as a percentage. */
  min?: number;
  children: [React.ReactNode, React.ReactNode];
}

export function SplitPane({
  direction,
  initial,
  min = 10,
  children,
}: SplitPaneProps): JSX.Element {
  const [size, setSize] = useState(initial);
  const container = useRef<HTMLDivElement>(null);
  const horizontal = direction === "horizontal";

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const move = (ev: PointerEvent) => {
        const el = container.current;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        const pct = horizontal
          ? ((ev.clientX - rect.left) / rect.width) * 100
          : ((ev.clientY - rect.top) / rect.height) * 100;
        setSize(Math.max(min, Math.min(100 - min, pct)));
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [horizontal, min]
  );

  return (
    <div
      ref={container}
      className={`split split-${direction}`}
      style={{ flexDirection: horizontal ? "row" : "column" }}
    >
      <div className="split-pane" style={{ flexBasis: `${size}%` }}>
        {children[0]}
      </div>
      <div
        className={`split-divider split-divider-${direction}`}
        onPointerDown={onPointerDown}
        role="separator"
        aria-orientation={horizontal ? "vertical" : "horizontal"}
      />
      <div className="split-pane" style={{ flexBasis: `${100 - size}%` }}>
        {children[1]}
      </div>
    </div>
  );
}
