import { useRef, useState } from "react";
import { fileTime, type FileRow } from "./browse-api";
import RenameInput from "./RenameInput";

export default function FileList({
  rows,
  selection,
  onSelect,
  onOpen,
  next,
  busy,
  onMore,
  allSelected = false,
  onSelectAll,
  onContext,
  onFileKey,
  onDrag,
  onBlankContext,
  editing,
}: {
  rows: FileRow[];
  selection: string[];
  onSelect: (ids: string[], all?: boolean) => void;
  onOpen: (row: FileRow) => void;
  next: number | null;
  busy: boolean;
  onMore: () => void;
  allSelected?: boolean;
  onSelectAll?: () => void;
  onContext?: (row: FileRow, x: number, y: number) => void;
  onFileKey?: (e: React.KeyboardEvent) => boolean;
  onDrag?: (row: FileRow, current: boolean) => void;
  onBlankContext?: (x: number, y: number) => void;
  editing?: {
    entry: string;
    onSubmit: (name: string) => void;
    onCancel: () => void;
  };
}) {
  const anchor = useRef(0);
  const dragging = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  const suppressClick = useRef(false);
  const selected = (row: FileRow) =>
    allSelected
      ? !selection.includes(row.entry.join(","))
      : selection.includes(row.entry.join(","));
  const [top, setTop] = useState(0);
  const start = Math.max(0, Math.floor(top / 34) - 10);
  const end = Math.min(rows.length, start + 50);
  function choose(index: number, extend: boolean, toggle: boolean) {
    const id = rows[index].entry.join(",");
    if (extend) {
      onSelect(
        rows
          .slice(
            Math.min(anchor.current, index),
            Math.max(anchor.current, index) + 1,
          )
          .map((r) => r.entry.join(",")),
      );
    } else {
      anchor.current = index;
      onSelect(
        toggle
          ? selection.includes(id)
            ? selection.filter((x) => x !== id)
            : [...selection, id]
          : [id],
        toggle ? allSelected : false,
      );
    }
  }
  return (
    <div
      className="directory-list"
      aria-label="目录文件列表"
      onScroll={(e) => setTop(e.currentTarget.scrollTop)}
      onContextMenu={(e) => {
        if (
          onBlankContext &&
          !(e.target as HTMLElement).closest("tr[data-index]")
        ) {
          e.preventDefault();
          onBlankContext(e.clientX, e.clientY);
        }
      }}
      tabIndex={0}
      onKeyDown={(e) => {
        if (!e.defaultPrevented && onFileKey?.(e)) return;
        if (e.ctrlKey && e.key.toLowerCase() === "a") {
          e.preventDefault();
          if (onSelectAll) onSelectAll();
          else onSelect(rows.map((r) => r.entry.join(",")));
        }
      }}
    >
      <table aria-label="文件" aria-multiselectable="true">
        <thead>
          <tr>
            <th>名称</th>
            <th>大小</th>
            <th>修改时间</th>
          </tr>
        </thead>
        <tbody>
          {start > 0 && (
            <tr aria-hidden="true" style={{ height: start * 34 }}>
              <td colSpan={3} />
            </tr>
          )}
          {rows.slice(start, end).map((row, local) => {
            const index = start + local;
            return (
              <tr
                key={row.entry.join(",")}
                tabIndex={0}
                data-index={index}
                aria-selected={selected(row)}
                onClick={(e) => {
                  if (suppressClick.current) {
                    suppressClick.current = false;
                    return;
                  }
                  choose(index, e.shiftKey, e.ctrlKey);
                }}
                onContextMenu={(e) => {
                  if (!onContext) return;
                  e.preventDefault();
                  e.currentTarget.focus();
                  if (!selected(row)) onSelect([row.entry.join(",")], false);
                  onContext(row, e.clientX, e.clientY);
                }}
                onPointerDown={(e) => {
                  if (e.button === 0) {
                    suppressClick.current = false;
                    dragging.current = {
                      x: e.clientX,
                      y: e.clientY,
                      started: false,
                    };
                  }
                }}
                onPointerMove={(e) => {
                  const start = dragging.current;
                  if (
                    !onDrag ||
                    !start ||
                    start.started ||
                    e.buttons !== 1 ||
                    Math.hypot(e.clientX - start.x, e.clientY - start.y) < 8
                  )
                    return;
                  start.started = true;
                  suppressClick.current = true;
                  e.preventDefault();
                  onDrag(row, selected(row));
                }}
                onPointerUp={() => {
                  dragging.current = null;
                }}
                onDragStart={(e) => e.preventDefault()}
                onDoubleClick={() => onOpen(row)}
                onKeyDown={(e) => {
                  if (onFileKey?.(e)) return;
                  if (e.key === "Enter") {
                    e.preventDefault();
                    onOpen(row);
                  }
                  if (e.key === " ") {
                    e.preventDefault();
                    choose(index, e.shiftKey, e.ctrlKey);
                  }
                  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                    e.preventDefault();
                    const target = Math.max(
                      0,
                      Math.min(
                        rows.length - 1,
                        index + (e.key === "ArrowDown" ? 1 : -1),
                      ),
                    );
                    choose(target, e.shiftKey, false);
                    const sibling =
                      e.currentTarget.parentElement?.querySelector(
                        `[data-index="${target}"]`,
                      ) as HTMLElement | undefined;
                    if (!sibling) {
                      const list = e.currentTarget.closest(
                        ".directory-list",
                      ) as HTMLElement;
                      list.scrollTop = target * 34;
                      setTop(target * 34);
                      requestAnimationFrame(() =>
                        list
                          .querySelector<HTMLElement>(
                            `[data-index="${target}"]`,
                          )
                          ?.focus(),
                      );
                    }
                    sibling?.focus();
                  }
                }}
              >
                <td title={row.name}>
                  <span
                    className={`file-mark ${row.kind}`}
                    aria-hidden="true"
                  />
                  {editing?.entry === row.entry.join(",") ? (
                    <RenameInput
                      name={row.name}
                      folder={row.kind === "folder"}
                      onSubmit={editing.onSubmit}
                      onCancel={editing.onCancel}
                    />
                  ) : (
                    row.name
                  )}
                </td>
                <td className="result-size">
                  {row.kind === "folder"
                    ? "—"
                    : row.size == null
                      ? "未知"
                      : `${BigInt(row.size).toLocaleString()} B`}
                </td>
                <td>{fileTime(row.modified)}</td>
              </tr>
            );
          })}
          {end < rows.length && (
            <tr aria-hidden="true" style={{ height: (rows.length - end) * 34 }}>
              <td colSpan={3} />
            </tr>
          )}
        </tbody>
      </table>
      {!busy && !rows.length && (
        <p className="search-empty">此目录没有已索引的文件</p>
      )}
      {next !== null && (
        <button className="load-more" disabled={busy} onClick={onMore}>
          加载更多文件
        </button>
      )}
    </div>
  );
}
