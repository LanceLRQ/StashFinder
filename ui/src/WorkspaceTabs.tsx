import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

export default function WorkspaceTabs({
  active,
  home,
  onAdd,
  children,
}: {
  active: number;
  home?: ReactNode;
  onAdd: () => void;
  children: ReactNode;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const track = useRef<HTMLDivElement>(null);
  const previous = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const [scroll, setScroll] = useState({
    overflow: false,
    left: false,
    right: false,
  });

  function measure() {
    const area = viewport.current!;
    const hidden = area.scrollWidth - area.clientWidth;
    const arrows =
      (previous.current?.offsetWidth ?? 0) + (next.current?.offsetWidth ?? 0);
    const overflow = hidden > arrows + 1;
    const left = area.scrollLeft > 1;
    const right = hidden - area.scrollLeft > 1;
    setScroll((old) =>
      old.overflow === overflow && old.left === left && old.right === right
        ? old
        : { overflow, left, right },
    );
  }

  useLayoutEffect(() => {
    const area = viewport.current!;
    const tab = track.current?.querySelector<HTMLButtonElement>(
      '[aria-selected="true"]',
    );
    const item = tab?.parentElement;
    if (item && area.clientWidth) {
      const left = item.offsetLeft;
      const right = left + item.offsetWidth;
      if (left < area.scrollLeft) area.scrollLeft = left;
      else if (right > area.scrollLeft + area.clientWidth)
        area.scrollLeft = right - area.clientWidth;
    }
    measure();
  }, [active, scroll.overflow]);

  useLayoutEffect(() => {
    measure();
    window.addEventListener("resize", measure);
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(measure);
    observer?.observe(viewport.current!);
    observer?.observe(track.current!);
    return () => {
      window.removeEventListener("resize", measure);
      observer?.disconnect();
    };
  }, []);

  function page(direction: number) {
    const area = viewport.current!;
    area.scrollBy({ left: direction * area.clientWidth });
  }

  return (
    <div className="workspace-tab-strip">
      {scroll.overflow && (
        <button
          type="button"
          className="tab-scroll"
          ref={previous}
          aria-label="向左滚动标签页"
          disabled={!scroll.left}
          onClick={() => page(-1)}
        >
          ‹
        </button>
      )}
      <div className="workspace-tabs" role="tablist" aria-label="工作区标签页">
        {home}
        <div
          className="workspace-tab-viewport"
          ref={viewport}
          onScroll={measure}
        >
          <div className="workspace-tab-track" ref={track}>
            {children}
          </div>
        </div>
      </div>
      {scroll.overflow && (
        <button
          type="button"
          className="tab-scroll"
          ref={next}
          aria-label="向右滚动标签页"
          disabled={!scroll.right}
          onClick={() => page(1)}
        >
          ›
        </button>
      )}
      <button
        type="button"
        className="tab-new"
        aria-label="新建浏览标签页"
        title="新建浏览标签页 (Ctrl+T)"
        onClick={onAdd}
      >
        +
      </button>
    </div>
  );
}
