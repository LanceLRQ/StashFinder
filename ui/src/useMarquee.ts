import { useLayoutEffect, useRef, useState, type PointerEvent } from "react";
import { detailsGutter, tileInset, type ItemRectangle } from "./marquee";

type Point = { x: number; y: number };
type Frame = { area: ItemRectangle | null; toggle: boolean };
export type MarqueeCommit = (
  area: ItemRectangle | null,
  toggle: boolean,
) => void;

export default function useMarquee(options: {
  enabled: boolean;
  version: string | number | null | undefined;
  columns: number;
  rowHeight: number;
  tileWidth?: number;
  total: number;
  measure: (element: HTMLDivElement) => { offset: number; header: number };
  scroll: (element: HTMLDivElement, delta: number) => boolean;
  page?: { offset: number; load: () => void };
  commit?: MarqueeCommit;
}) {
  const latest = useRef(options);
  const drag = useRef<{
    id: number;
    element: HTMLDivElement;
    from: Point;
    pointer: Point;
    origin: Point;
    toggle: boolean;
    moved: boolean;
    page?: number;
    animation?: number;
  } | null>(null);
  const [frame, setFrame] = useState<
    | (Frame & {
        box: { left: number; top: number; width: number; height: number };
      })
    | null
  >(null);
  useLayoutEffect(() => {
    latest.current = options;
  });
  useLayoutEffect(
    () => () => {
      cancel();
    },
    [options.enabled, options.version, options.columns, options.rowHeight],
  );

  function cancel() {
    const current = drag.current;
    if (!current) return false;
    drag.current = null;
    if (current.animation !== undefined)
      cancelAnimationFrame(current.animation);
    if (current.element.hasPointerCapture?.(current.id))
      current.element.releasePointerCapture(current.id);
    if (current.moved) setFrame(null);
    return true;
  }

  function bounds(element: HTMLDivElement) {
    const rect = element.getBoundingClientRect();
    const x = rect.left + element.clientLeft;
    const y = rect.top + element.clientTop;
    return {
      x,
      y,
      right: x + element.clientWidth,
      bottom: y + element.clientHeight,
    };
  }

  function sample(current: NonNullable<typeof drag.current>) {
    const { element, origin, pointer, toggle } = current;
    const opts = latest.current;
    const rect = bounds(element);
    const { offset, header } = opts.measure(element);
    const clip = (n: number, min: number, max: number) =>
      Math.max(min, Math.min(max, n));
    const yMin = Math.min(rect.bottom, rect.y + header);
    const x = clip(pointer.x, rect.x, rect.right);
    const y = clip(pointer.y, yMin, rect.bottom);
    const ax = origin.x - element.scrollLeft + rect.x;
    const ay = origin.y - offset + rect.y;
    const left = Math.min(x, ax) - rect.x + element.scrollLeft;
    const right = Math.max(x, ax) - rect.x + element.scrollLeft;
    const top = Math.min(y, ay) - rect.y + offset;
    const bottom = Math.max(y, ay) - rect.y + offset;
    const { rowHeight, columns, tileWidth, total } = opts;
    const inset = tileWidth ? tileInset : 0;
    const firstRow = Math.max(0, Math.floor((top + inset) / rowHeight));
    const lastRow = Math.min(
      Math.ceil(total / columns) - 1,
      Math.ceil((bottom - inset) / rowHeight) - 1,
    );
    const firstColumn = tileWidth
      ? Math.max(0, Math.floor((left + inset) / tileWidth))
      : 0;
    const lastColumn = tileWidth
      ? Math.min(columns - 1, Math.ceil((right - inset) / tileWidth) - 1)
      : Number(right > detailsGutter) - 1;
    const area =
      firstRow <= lastRow && firstColumn <= lastColumn
        ? { firstRow, lastRow, firstColumn, lastColumn, columns }
        : null;
    const sx = clip(ax, rect.x, rect.right);
    const sy = clip(ay, yMin, rect.bottom);
    return {
      area,
      toggle,
      box: {
        left: Math.min(x, sx),
        top: Math.min(y, sy),
        width: Math.abs(x - sx),
        height: Math.abs(y - sy),
      },
    };
  }

  function refresh() {
    if (drag.current?.moved) setFrame(sample(drag.current));
  }

  function tick() {
    const current = drag.current;
    if (!current?.moved) return;
    current.animation = undefined;
    const { y, bottom } = bounds(current.element);
    const edge = Math.min(32, (bottom - y) / 3);
    const delta =
      current.pointer.y < y + edge
        ? Math.max(-28, current.pointer.y - y - edge)
        : current.pointer.y > bottom - edge
          ? Math.min(28, current.pointer.y - bottom + edge)
          : 0;
    if (!delta) return;
    const opts = latest.current;
    const previousTop = current.element.scrollTop;
    const atEnd = opts.scroll(current.element, delta);
    const page = opts.page;
    if (delta > 0 && atEnd && page && current.page !== page.offset) {
      current.page = page.offset;
      page.load();
    }
    if (current.element.scrollTop !== previousTop) refresh();
    current.animation = requestAnimationFrame(tick);
  }

  return {
    frame,
    cancel,
    refresh,
    isActive: () => drag.current !== null,
    handlers: {
      onPointerDown(e: PointerEvent<HTMLDivElement>) {
        const target = e.target as HTMLElement;
        const opts = latest.current;
        if (
          !opts.enabled ||
          !opts.commit ||
          drag.current ||
          e.button !== 0 ||
          e.pointerType === "touch" ||
          target.closest(
            "[data-index], thead, button, input, textarea, select, a",
          )
        )
          return;
        const element = e.currentTarget;
        const rect = bounds(element);
        const { offset, header } = opts.measure(element);
        if (
          e.clientX < rect.x ||
          e.clientX >= rect.right ||
          e.clientY < rect.y + header ||
          e.clientY >= rect.bottom
        )
          return;
        const point = { x: e.clientX, y: e.clientY };
        drag.current = {
          id: e.pointerId,
          element,
          from: point,
          pointer: point,
          origin: {
            x: point.x - rect.x + element.scrollLeft,
            y: point.y - rect.y + offset,
          },
          toggle: e.ctrlKey,
          moved: false,
        };
        e.preventDefault();
        element.focus({ preventScroll: true });
        element.setPointerCapture?.(e.pointerId);
      },
      onPointerMove(e: PointerEvent<HTMLDivElement>) {
        const current = drag.current;
        if (!current || e.pointerId !== current.id) return;
        if (!(e.buttons & 1)) {
          cancel();
          return;
        }
        current.pointer = { x: e.clientX, y: e.clientY };
        if (
          !current.moved &&
          Math.hypot(e.clientX - current.from.x, e.clientY - current.from.y) < 5
        )
          return;
        current.moved = true;
        refresh();
        if (current.animation === undefined)
          current.animation = requestAnimationFrame(tick);
      },
      onPointerUp(e: PointerEvent<HTMLDivElement>) {
        const current = drag.current;
        if (!current || e.pointerId !== current.id) return;
        if (current.moved) {
          current.pointer = { x: e.clientX, y: e.clientY };
          const { area, toggle } = sample(current);
          latest.current.commit?.(area, toggle);
        }
        cancel();
      },
      onPointerCancel(e: PointerEvent<HTMLDivElement>) {
        if (e.pointerId === drag.current?.id) cancel();
      },
      onLostPointerCapture(e: PointerEvent<HTMLDivElement>) {
        if (e.pointerId === drag.current?.id) cancel();
      },
    },
  };
}
