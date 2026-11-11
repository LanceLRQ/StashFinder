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
import { detailsGutter, hitsItem, tileInset } from "./marquee";
import useMarquee, { type MarqueeCommit } from "./useMarquee";
import {
  DetailsHeader,
  fitColumnWidths,
  useColumnSettings,
} from "./DetailsColumns";
import type { FileColumn } from "./file-columns";
import { fileEditingKey } from "./file-operations";

const maxScrollHeight = 8_000_000;
const initialHeight = 500;

type DetailsLayout = { rowHeight: number; overscan: number };
type GridLayout = { rowHeight: number; tileWidth: number };
export type ViewportLayout = DetailsLayout & { columns: number };

export function virtualScale(
  total: number,
  height: number,
  rowHeight: number,
  columns = 1,
) {
  const logicalHeight = Math.ceil(total / columns) * rowHeight;
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
  { rowHeight, overscan, columns = 1 }: DetailsLayout & { columns?: number },
) {
  const rows = Math.ceil(total / columns);
  const content = Math.min(rows * rowHeight, maxScrollHeight);
  const scale = virtualScale(total, height, rowHeight, columns);
  const clamped = Math.max(0, Math.min(logicalTop, rows * rowHeight - height));
  const top = clamped / scale;
  const start = Math.max(0, Math.floor(clamped / rowHeight) - overscan);
  const end = Math.min(
    rows,
    start + Math.ceil(height / rowHeight) + 2 * overscan,
  );
  const rowsHeight = (end - start) * rowHeight;
  // Round once so Chromium's independent spacer rounding cannot change the scroll height.
  const before = Math.max(
    0,
    Math.min(
      Math.round(top - (clamped - start * rowHeight)),
      content - rowsHeight,
    ),
  );
  const after = Math.max(0, content - before - rowsHeight);
  return {
    start: start * columns,
    end: Math.min(total, end * columns),
    top,
    logicalTop: clamped,
    before,
    after,
  };
}

export default function DetailsViewport({
  className,
  label,
  tableLabel,
  layout,
  grid,
  columns,
  active = true,
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
  marquee,
  row,
  children,
}: {
  className: string;
  label: string;
  tableLabel?: string;
  layout: DetailsLayout;
  grid?: GridLayout;
  columns: readonly FileColumn[];
  active?: boolean;
  total: number;
  revision?: number;
  restore?: { logicalTop: number } | null;
  onRange?: (start: number, end: number) => void;
  onPosition?: (
    logicalTop: number,
    height: number,
    scrolling: boolean,
    layout: ViewportLayout,
  ) => void;
  onSort?: (field: string) => void;
  sort?: string;
  descending?: boolean;
  onKeyDown: (
    event: KeyboardEvent<HTMLDivElement>,
    navigation: {
      columns: number;
      reveal: (index: number) => void;
      scrollTo: (top: number) => void;
    },
  ) => void;
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  marquee?: {
    enabled: boolean;
    version?: string | number | null;
    commit: MarqueeCommit;
    page?: { offset: number; load: () => void };
  };
  row: (
    index: number,
    scrollToRow: (index: number) => void,
    columns: number,
    frame: { hit: boolean; toggle: boolean } | undefined,
    detailColumns: readonly FileColumn[],
  ) => ReactNode;
  children?: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const table = useRef<HTMLTableElement>(null);
  const columnSettings = useColumnSettings(columns, className);
  const visibleColumns = columnSettings.visible;
  const focused = useRef<HTMLElement | null>(null);
  const pendingFocus = useRef<string | undefined>(undefined);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(initialHeight);
  const [width, setWidth] = useState(0);
  const rowHeight = grid?.rowHeight ?? layout.rowHeight;
  const itemsPerRow = grid
    ? Math.max(1, Math.floor(width / grid.tileWidth))
    : 1;
  const overscan = grid ? 1 : layout.overscan;
  const geometry = { rowHeight, overscan, columns: itemsPerRow };
  const scale = virtualScale(total, height, rowHeight, itemsPerRow);
  const previous = useRef<{
    rowHeight: number;
    columns: number;
    height: number;
    revision?: number;
    restore: typeof restore;
  } | null>(null);
  const anchor = useRef({ index: 0, physicalTop: 0 });
  const { start, end, before, after } = virtualWindow(
    total,
    top * scale,
    height,
    geometry,
  );
  const frame = useMarquee({
    enabled: !!marquee?.enabled,
    version: marquee?.version,
    commit: marquee?.commit,
    page: marquee?.page,
    columns: itemsPerRow,
    rowHeight,
    tileWidth: grid?.tileWidth,
    total,
    measure(element) {
      const header = grid
        ? 0
        : (element.querySelector("thead")?.offsetHeight ?? 0);
      const window = virtualWindow(
        total,
        element.scrollTop * scale,
        height,
        geometry,
      );
      return {
        header,
        offset:
          (window.start / itemsPerRow) * rowHeight +
          element.scrollTop -
          window.before -
          header,
      };
    },
    scroll(element, delta) {
      const header = grid
        ? 0
        : (element.querySelector("thead")?.offsetHeight ?? 0);
      const content = Math.min(
        Math.ceil(total / itemsPerRow) * rowHeight,
        maxScrollHeight,
      );
      const max = Math.max(0, content + header - height);
      const previous = element.scrollTop;
      const step = Math.sign(delta) * Math.max(1, Math.abs(delta) / scale);
      const next = Math.max(0, Math.min(max, previous + step));
      if (next !== previous) {
        element.scrollTop = next;
        updatePosition(element, true);
      }
      return element.scrollTop >= max - 1;
    },
  });

  function updatePosition(element: HTMLDivElement, scrolling: boolean) {
    const { scrollTop, clientHeight } = element;
    const viewportHeight = clientHeight || height;
    setTop(scrollTop);
    setHeight(viewportHeight);
    const logicalTop =
      scrollTop * virtualScale(total, viewportHeight, rowHeight, itemsPerRow);
    if (Math.abs(scrollTop - anchor.current.physicalTop) > 0.5)
      anchor.current = {
        index: Math.floor(logicalTop / rowHeight) * itemsPerRow,
        physicalTop: scrollTop,
      };
    onPosition?.(logicalTop, viewportHeight, scrolling, geometry);
  }

  function scrollTo(logicalTop: number) {
    const element = ref.current;
    if (!element) return;
    const viewportHeight = element.clientHeight || height;
    element.scrollTop = virtualWindow(
      total,
      logicalTop,
      viewportHeight,
      geometry,
    ).top;
    updatePosition(element, false);
  }

  function scrollToRow(index: number) {
    scrollTo(Math.floor(index / itemsPerRow) * rowHeight);
  }

  function reveal(index: number) {
    const rowTop = Math.floor(index / itemsPerRow) * rowHeight;
    const logicalTop = (ref.current?.scrollTop ?? top) * scale;
    if (rowTop < logicalTop) scrollTo(rowTop);
    else if (rowTop + rowHeight > logicalTop + height)
      scrollTo(grid ? rowTop + rowHeight - height : rowTop);
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
    const element = ref.current!;
    const old = previous.current;
    previous.current = {
      rowHeight,
      columns: itemsPerRow,
      height,
      revision,
      restore,
    };
    const reflow =
      !!old &&
      (old.rowHeight !== rowHeight ||
        old.columns !== itemsPerRow ||
        old.height !== height);
    if (!old || old.revision !== revision) {
      element.scrollTop = 0;
      anchor.current = { index: 0, physicalTop: 0 };
    } else if (restore && old.restore !== restore) {
      element.scrollTop = virtualWindow(
        total,
        restore.logicalTop,
        height,
        geometry,
      ).top;
      anchor.current = {
        index: Math.floor(restore.logicalTop / rowHeight) * itemsPerRow,
        physicalTop: element.scrollTop,
      };
    } else if (reflow) {
      const index = anchor.current.index;
      element.scrollTop = virtualWindow(
        total,
        Math.floor(index / itemsPerRow) * rowHeight,
        height,
        geometry,
      ).top;
      anchor.current = { index, physicalTop: element.scrollTop };
    }
    updatePosition(element, false);
    const item = focused.current;
    if (
      reflow &&
      item &&
      !item.isConnected &&
      document.activeElement === document.body
    ) {
      pendingFocus.current = item.dataset.index;
    }
  }, [
    height,
    itemsPerRow,
    overscan,
    restore,
    revision,
    rowHeight,
    scale,
    total,
  ]);
  useLayoutEffect(() => {
    const index = pendingFocus.current;
    const element = ref.current!;
    if (index === undefined || top !== element.scrollTop) return;
    pendingFocus.current = undefined;
    const target = element.querySelector<HTMLElement>(
      `[data-index="${index}"]`,
    );
    (target ?? element).focus({ preventScroll: true });
  }, [start, end, top, rowHeight, itemsPerRow]);
  useEffect(() => {
    onRange?.(start, end);
  }, [start, end, onRange]);
  useLayoutEffect(() => {
    const element = ref.current!;
    const measure = () => {
      setHeight((previous) => element.clientHeight || previous);
      setWidth((previous) => element.clientWidth || previous);
    };
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const spacer = (space: number) =>
    space > 0 && (
      <tr
        className="details-spacer"
        aria-hidden="true"
        style={{ height: space }}
      >
        <td colSpan={visibleColumns.length} />
      </tr>
    );
  const items = () =>
    Array.from({ length: end - start }, (_, offset) => {
      const index = start + offset;
      return row(
        index,
        scrollToRow,
        itemsPerRow,
        frame.frame
          ? {
              hit: hitsItem(frame.frame.area, index),
              toggle: frame.frame.toggle,
            }
          : undefined,
        visibleColumns,
      );
    });

  return (
    <div
      className={`details-viewport ${className}`}
      aria-label={label}
      style={
        {
          "--details-row-height": `${rowHeight}px`,
          "--details-gutter": `${detailsGutter}px`,
          "--tile-inset": `${tileInset}px`,
        } as CSSProperties
      }
      ref={ref}
      tabIndex={0}
      onScroll={(event) => {
        updatePosition(event.currentTarget, true);
        frame.refresh();
      }}
      onFocus={(event) => {
        focused.current = (event.target as HTMLElement).closest("[data-index]");
      }}
      onKeyDown={(event) => {
        if (frame.isActive()) {
          if (event.key === "Tab") frame.cancel();
          else {
            if (event.key === "Escape") frame.cancel();
            event.preventDefault();
            event.stopPropagation();
            return;
          }
        }
        if (
          !grid &&
          event.ctrlKey &&
          event.code === "NumpadAdd" &&
          !fileEditingKey(event)
        ) {
          event.preventDefault();
          columnSettings.resize(fitColumnWidths(table.current, visibleColumns));
          return;
        }
        onKeyDown(event, { columns: itemsPerRow, reveal, scrollTo });
      }}
      onContextMenu={onContextMenu}
      {...frame.handlers}
    >
      {grid ? (
        <div
          className="thumbnail-list"
          role="listbox"
          aria-label={tableLabel ?? label}
          aria-multiselectable="true"
        >
          <div aria-hidden="true" style={{ height: before }} />
          <div
            className="thumbnail-cells"
            role="presentation"
            style={{
              gridTemplateColumns: `repeat(${itemsPerRow}, minmax(0, ${grid.tileWidth}px))`,
            }}
          >
            {items()}
          </div>
          <div aria-hidden="true" style={{ height: after }} />
        </div>
      ) : (
        <table
          className="marquee-details"
          ref={table}
          style={{
            width: visibleColumns.reduce(
              (width, column) => width + column.pixels,
              0,
            ),
          }}
          aria-label={tableLabel}
          aria-rowcount={total + 1}
          aria-multiselectable="true"
        >
          <colgroup>
            {visibleColumns.map((column) => (
              <col key={column.id} style={{ width: column.pixels }} />
            ))}
          </colgroup>
          <DetailsHeader
            columns={columns}
            settings={columnSettings}
            table={table}
            sort={sort}
            descending={descending}
            onSort={onSort}
            active={active}
          />
          <tbody>
            {spacer(before)}
            {items()}
            {spacer(after)}
          </tbody>
        </table>
      )}
      {children}
      {frame.frame && (
        <div
          className="file-marquee"
          aria-hidden="true"
          style={frame.frame.box}
        />
      )}
    </div>
  );
}
