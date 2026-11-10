import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import "./details-viewport.css";

const maxScrollHeight = 8_000_000;
const initialHeight = 500;

type DetailsLayout = { rowHeight: number; overscan: number };

export function virtualScale(total: number, height: number, rowHeight: number) {
  const logicalHeight = total * rowHeight;
  const contentHeight = Math.min(logicalHeight, maxScrollHeight);
  return Math.max(
    1,
    (logicalHeight - height) / Math.max(1, contentHeight - height),
  );
}

export function virtualWindow(
  total: number,
  logicalTop: number,
  height: number,
  { rowHeight, overscan }: DetailsLayout,
) {
  const content = Math.min(total * rowHeight, maxScrollHeight);
  const scale = virtualScale(total, height, rowHeight);
  const clamped = Math.max(0, Math.min(logicalTop, total * rowHeight - height));
  const top = clamped / scale;
  const start = Math.max(0, Math.floor(clamped / rowHeight) - overscan);
  const end = Math.min(
    total,
    start + Math.ceil(height / rowHeight) + 2 * overscan,
  );
  const rowsHeight = (end - start) * rowHeight;
  // Keep the fractional row offset while the physical scrollbar spans the whole result set.
  const before = Math.max(
    0,
    Math.min(top - (clamped - start * rowHeight), content - rowsHeight),
  );
  const after = Math.max(0, content - before - rowsHeight);
  return { start, end, top, logicalTop: clamped, before, after };
}

export default function DetailsViewport({
  className,
  label,
  tableLabel,
  layout,
  columns,
  total,
  revision,
  restore,
  onRange,
  onPosition,
  onSort,
  sort,
  descending,
  onKeyDown,
  onContextMenu,
  row,
  children,
}: {
  className: string;
  label: string;
  tableLabel?: string;
  layout: DetailsLayout;
  columns: { label: string; sort?: string }[];
  total: number;
  revision?: number;
  restore?: { logicalTop: number } | null;
  onRange?: (start: number, end: number) => void;
  onPosition?: (logicalTop: number, height: number, scrolling: boolean) => void;
  onSort?: (field: string) => void;
  sort?: string;
  descending?: boolean;
  onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void;
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  row: (index: number, scrollToRow: (index: number) => void) => ReactNode;
  children?: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(initialHeight);
  const scale = virtualScale(total, height, layout.rowHeight);
  const { start, end, before, after } = virtualWindow(
    total,
    top * scale,
    height,
    layout,
  );

  function updatePosition(element: HTMLDivElement, scrolling: boolean) {
    const { scrollTop, clientHeight } = element;
    const viewportHeight = clientHeight || height;
    setTop(scrollTop);
    setHeight(viewportHeight);
    onPosition?.(
      scrollTop * virtualScale(total, viewportHeight, layout.rowHeight),
      viewportHeight,
      scrolling,
    );
  }

  function scrollTo(logicalTop: number) {
    const element = ref.current;
    if (!element) return;
    const viewportHeight = element.clientHeight || height;
    element.scrollTop = virtualWindow(
      total,
      logicalTop,
      viewportHeight,
      layout,
    ).top;
    updatePosition(element, false);
  }

  function scrollToRow(index: number) {
    scrollTo(index * layout.rowHeight);
  }

  useEffect(() => {
    const element = ref.current;
    if (!element || scale === 1) return;
    const wheel = (event: WheelEvent) => {
      if (event.deltaY) {
        event.preventDefault();
        element.scrollTop += event.deltaY / scale;
      }
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [scale]);
  useLayoutEffect(() => {
    setTop(0);
    if (ref.current) ref.current.scrollTop = 0;
  }, [revision]);
  useLayoutEffect(() => {
    if (restore) scrollTo(restore.logicalTop);
  }, [restore, total, layout]);
  useEffect(() => {
    onRange?.(start, end);
  }, [start, end, onRange]);
  useEffect(() => {
    const element = ref.current!;
    const measure = () =>
      setHeight((previous) => element.clientHeight || previous);
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const element = ref.current;
    if (element) onPosition?.(element.scrollTop * scale, height, false);
  }, [height, revision]);

  const spacer = (space: number) =>
    space > 0 && (
      <tr
        className="details-spacer"
        aria-hidden="true"
        style={{ height: space }}
      >
        <td colSpan={columns.length} />
      </tr>
    );

  return (
    <div
      className={`details-viewport ${className}`}
      aria-label={label}
      style={
        { "--details-row-height": `${layout.rowHeight}px` } as CSSProperties
      }
      ref={ref}
      tabIndex={0}
      onScroll={(event) => updatePosition(event.currentTarget, true)}
      onKeyDown={onKeyDown}
      onContextMenu={onContextMenu}
    >
      <table
        aria-label={tableLabel}
        aria-rowcount={total + 1}
        aria-multiselectable="true"
      >
        <thead>
          <tr>
            {columns.map(({ label, sort: field }) => (
              <th
                key={label}
                scope="col"
                aria-sort={
                  field && field === sort
                    ? descending
                      ? "descending"
                      : "ascending"
                    : undefined
                }
              >
                {field ? (
                  <button
                    aria-label={`按${label}排序`}
                    onClick={() => onSort?.(field)}
                  >
                    {label}
                  </button>
                ) : (
                  label
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {spacer(before)}
          {Array.from({ length: end - start }, (_, offset) =>
            row(start + offset, scrollToRow),
          )}
          {spacer(after)}
        </tbody>
      </table>
      {children}
    </div>
  );
}
