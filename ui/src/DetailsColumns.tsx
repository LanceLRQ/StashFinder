import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import type { FileColumn, FileColumnId } from "./file-columns";

const minWidth = 64;
const maxWidth = 1600;
const changedEvent = "stashfinder-details-columns";
const clampWidth = (width: number) =>
  Math.max(minWidth, Math.min(maxWidth, Math.round(width)));

type ColumnSettings = {
  hidden?: Partial<Record<FileColumnId, boolean>>;
  widths?: Partial<Record<FileColumnId, number>>;
};
type VisibleColumn = FileColumn & { pixels: number };

function readSettings(key: string): ColumnSettings {
  try {
    const value = JSON.parse(localStorage.getItem(key) || "{}");
    return value && typeof value === "object" ? value : {};
  } catch {
    return {};
  }
}

export function useColumnSettings(
  columns: readonly FileColumn[],
  scope: string,
) {
  const key = `stashfinder.details.${scope}.v1`;
  const [settings, setSettings] = useState(() => readSettings(key));
  const [draft, setDraft] = useState<{
    id: FileColumnId;
    width: number;
  } | null>(null);
  useEffect(() => {
    function update(event: Event) {
      if ((event as CustomEvent<string>).detail === key)
        setSettings(readSettings(key));
    }
    window.addEventListener(changedEvent, update);
    return () => window.removeEventListener(changedEvent, update);
  }, [key]);

  function save(next: ColumnSettings) {
    setSettings(next);
    try {
      localStorage.setItem(key, JSON.stringify(next));
      window.dispatchEvent(new CustomEvent(changedEvent, { detail: key }));
    } catch {
      // The layout remains usable when the webview storage is unavailable.
    }
  }
  const visible = columns
    .filter((column) => {
      const hidden = settings.hidden?.[column.id];
      return (
        column.id === "name" ||
        !(typeof hidden === "boolean" ? hidden : column.optional)
      );
    })
    .map((column) => {
      const width = settings.widths?.[column.id];
      return {
        ...column,
        pixels:
          draft?.id === column.id
            ? draft.width
            : typeof width === "number" && Number.isFinite(width)
              ? clampWidth(width)
              : column.width,
      };
    });
  return {
    visible,
    toggle(id: FileColumnId, shown: boolean) {
      if (id !== "name")
        save({ ...settings, hidden: { ...settings.hidden, [id]: shown } });
    },
    preview(id: FileColumnId, width: number) {
      setDraft({ id, width: clampWidth(width) });
    },
    cancel() {
      setDraft(null);
    },
    resize(widths: NonNullable<ColumnSettings["widths"]>) {
      setDraft(null);
      save({ ...settings, widths: { ...settings.widths, ...widths } });
    },
  };
}

export function fitColumnWidths(
  table: HTMLTableElement | null,
  visible: VisibleColumn[],
  id?: FileColumnId,
) {
  if (!table) return {};
  const rows = table.querySelectorAll<HTMLTableRowElement>(
    "thead tr, tbody tr[data-index]",
  );
  const range = document.createRange();
  return Object.fromEntries(
    visible.flatMap((column, index) => {
      if (id && column.id !== id) return [];
      let widest = column.label.length * 13;
      for (const row of rows) {
        const cell = row.cells[index];
        if (!cell || cell.colSpan !== 1) continue;
        range.selectNodeContents(cell.querySelector("button") || cell);
        widest = Math.max(widest, range.getBoundingClientRect?.().width ?? 0);
      }
      return [[column.id, clampWidth(widest + 32)]];
    }),
  );
}

export function DetailsHeader({
  columns,
  settings,
  table,
  sort,
  descending,
  onSort,
  active,
}: {
  columns: readonly FileColumn[];
  settings: ReturnType<typeof useColumnSettings>;
  table: RefObject<HTMLTableElement | null>;
  sort?: string;
  descending?: boolean;
  onSort?: (field: string) => void;
  active: boolean;
}) {
  const { visible, toggle, preview, cancel, resize } = settings;
  const [menu, setMenu] = useState<{
    column: FileColumnId;
    x: number;
    y: number;
    anchor: HTMLElement;
  } | null>(null);
  const popup = useRef<HTMLDivElement>(null);
  const drag = useRef<AbortController | null>(null);
  useEffect(() => () => drag.current?.abort(), []);
  const menuButtons = () => [
    ...(popup.current?.querySelectorAll<HTMLButtonElement>(
      "button:not(:disabled)",
    ) ?? []),
  ];
  function closeMenu(restoreFocus: boolean) {
    setMenu(null);
    if (restoreFocus) menu?.anchor.focus({ preventScroll: true });
  }
  useEffect(() => {
    if (!active) setMenu(null);
  }, [active]);
  useLayoutEffect(() => {
    if (!menu || !popup.current) return;
    const rect = popup.current.getBoundingClientRect();
    popup.current.style.left = `${Math.max(0, Math.min(menu.x, innerWidth - rect.width))}px`;
    popup.current.style.top = `${Math.max(0, Math.min(menu.y, innerHeight - rect.height))}px`;
    menuButtons()[0]?.focus({ preventScroll: true });
  }, [menu]);
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      if (!popup.current?.contains(event.target as Node)) setMenu(null);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [menu]);
  function fit(id?: FileColumnId) {
    resize(fitColumnWidths(table.current, visible, id));
  }
  return (
    <thead>
      <tr>
        {visible.map((column) => (
          <th
            key={column.id}
            scope="col"
            aria-label={column.label}
            className={column.id === "size" ? "result-size" : undefined}
            tabIndex={column.sortable ? undefined : 0}
            aria-sort={
              column.sortable && sort === column.id
                ? descending
                  ? "descending"
                  : "ascending"
                : undefined
            }
            onContextMenu={(event) => {
              event.preventDefault();
              event.stopPropagation();
              const anchor =
                event.currentTarget.querySelector("button") ||
                event.currentTarget;
              setMenu({
                column: column.id,
                x: event.clientX,
                y: event.clientY,
                anchor,
              });
            }}
            onKeyDown={(event) => {
              if (
                event.key !== "ContextMenu" &&
                !(event.shiftKey && event.key === "F10")
              )
                return;
              event.preventDefault();
              event.stopPropagation();
              const { left, bottom } =
                event.currentTarget.getBoundingClientRect();
              setMenu({
                column: column.id,
                x: left,
                y: bottom,
                anchor: event.target as HTMLElement,
              });
            }}
          >
            {column.sortable ? (
              <button
                type="button"
                aria-label={`按${column.label}排序`}
                onClick={() => onSort?.(column.id)}
              >
                {column.label}
              </button>
            ) : (
              column.label
            )}
            <span
              className="column-resize"
              role="separator"
              tabIndex={0}
              aria-label={`调整${column.label}列宽`}
              aria-orientation="vertical"
              aria-valuemin={minWidth}
              aria-valuemax={maxWidth}
              aria-valuenow={column.pixels}
              onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.stopPropagation();
                drag.current?.abort();
                const controller = new AbortController();
                drag.current = controller;
                const { pointerId, clientX } = event;
                const listener = { signal: controller.signal, capture: true };
                const widthAt = (x: number) => column.pixels + x - clientX;
                const finish = () => {
                  controller.abort();
                  drag.current = null;
                  cancel();
                };
                window.addEventListener(
                  "pointermove",
                  (move) => {
                    if (move.pointerId !== pointerId) return;
                    if (move.buttons & 1)
                      preview(column.id, widthAt(move.clientX));
                    else finish();
                  },
                  listener,
                );
                window.addEventListener(
                  "pointerup",
                  (up) => {
                    if (up.pointerId !== pointerId) return;
                    finish();
                    if (up.clientX !== clientX)
                      resize({ [column.id]: clampWidth(widthAt(up.clientX)) });
                  },
                  listener,
                );
                window.addEventListener(
                  "pointercancel",
                  (canceled) => {
                    if (canceled.pointerId === pointerId) finish();
                  },
                  listener,
                );
                window.addEventListener("blur", finish, {
                  signal: controller.signal,
                });
                event.currentTarget.setPointerCapture?.(pointerId);
              }}
              onDoubleClick={() => fit(column.id)}
              onKeyDown={(event) => {
                if (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
                  return;
                event.preventDefault();
                event.stopPropagation();
                resize({
                  [column.id]: clampWidth(
                    column.pixels + (event.key === "ArrowRight" ? 8 : -8),
                  ),
                });
              }}
            />
          </th>
        ))}
      </tr>
      {active &&
        menu &&
        createPortal(
          <div
            className="details-column-menu"
            role="menu"
            aria-label="详情列"
            ref={popup}
            onKeyDown={(event) => {
              event.stopPropagation();
              if (event.key === "Escape") {
                event.preventDefault();
                closeMenu(true);
              } else if (event.key === "Tab") {
                closeMenu(true);
              } else if (
                ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
              ) {
                event.preventDefault();
                const buttons = menuButtons();
                const index = buttons.indexOf(
                  document.activeElement as HTMLButtonElement,
                );
                const next =
                  event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? buttons.length - 1
                      : (index +
                          (event.key === "ArrowDown" ? 1 : -1) +
                          buttons.length) %
                        buttons.length;
                buttons[next]?.focus();
              }
            }}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                fit(menu.column);
                closeMenu(true);
              }}
            >
              自适应当前列宽
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                fit();
                closeMenu(true);
              }}
            >
              自动调整列宽 <kbd>Ctrl+Num +</kbd>
            </button>
            <div role="separator" />
            {columns.map((column) => {
              const shown = visible.some(({ id }) => id === column.id);
              return (
                <button
                  type="button"
                  key={column.id}
                  role="menuitemcheckbox"
                  aria-checked={shown}
                  disabled={column.id === "name"}
                  onClick={() => {
                    toggle(column.id, shown);
                    closeMenu(true);
                  }}
                >
                  <span aria-hidden="true">{shown ? "✓" : ""}</span>
                  {column.label}
                </button>
              );
            })}
          </div>,
          document.body,
        )}
    </thead>
  );
}
