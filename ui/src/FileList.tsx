import { useRef } from "react";
import { fileTime, type FileRow } from "./browse-api";
import DetailsViewport from "./DetailsViewport";
import FileListItem from "./FileListItem";
import { fileEditingKey, type EntryRef } from "./file-operations";
import {
  fileDetailsLayout,
  thumbnailLayouts,
  type FileView,
} from "./file-view";
import { rectangleSpans } from "./marquee";

const columns = [{ label: "名称" }, { label: "大小" }, { label: "修改时间" }];

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
  context,
  connection,
  active = true,
  view = "details",
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
  context?: EntryRef;
  connection?: string;
  active?: boolean;
  view?: FileView;
}) {
  const grid = view === "details" ? undefined : thumbnailLayouts[view];
  const anchor = useRef(0);
  const dragging = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  const suppressClick = useRef(false);
  const selectedIds = new Set(selection);
  const selected = (row: FileRow) =>
    allSelected !== selectedIds.has(row.entry.join(","));
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
    <DetailsViewport
      className="directory-list"
      label="目录文件列表"
      tableLabel="文件"
      layout={fileDetailsLayout}
      grid={grid}
      columns={columns}
      total={rows.length}
      marquee={{
        enabled: active && !editing,
        version:
          context &&
          `${context.filesystem_id}:${context.generation}:${context.revision}`,
        page:
          next !== null && !busy ? { offset: next, load: onMore } : undefined,
        commit(area, toggle) {
          const spans = rectangleSpans(area, rows.length);
          const ids = new Set(toggle ? selection : []);
          if (spans.length) anchor.current = spans[0];
          for (let s = 0; s < spans.length; s += 2) {
            for (let i = spans[s]; i < spans[s + 1]; i++) {
              const id = rows[i].entry.join(",");
              if (!ids.delete(id)) ids.add(id);
            }
          }
          onSelect([...ids], toggle && allSelected);
        },
      }}
      onContextMenu={(e) => {
        if (
          onBlankContext &&
          !(e.target as HTMLElement).closest("[data-index]")
        ) {
          e.preventDefault();
          onBlankContext(e.clientX, e.clientY);
        }
      }}
      onKeyDown={(e) => {
        if (fileEditingKey(e) || onFileKey?.(e)) return;
        if (e.ctrlKey && e.key.toLowerCase() === "a") {
          e.preventDefault();
          if (onSelectAll) onSelectAll();
          else onSelect(rows.map((r) => r.entry.join(",")));
        }
      }}
      row={(index, scrollToRow, columns, frame) => {
        const row = rows[index];
        const marked = frame
          ? frame.toggle
            ? selected(row) !== frame.hit
            : frame.hit
          : selected(row);
        return (
          <FileListItem
            key={row.entry.join(",")}
            tabIndex={0}
            view={view}
            index={index}
            total={rows.length}
            selected={marked}
            file={row}
            active={active}
            reference={context ? { ...context, entry: row.entry } : undefined}
            connection={connection}
            editing={
              editing?.entry === row.entry.join(",") ? editing : undefined
            }
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
              if (fileEditingKey(e) || onFileKey?.(e)) return;
              if (e.key === "Enter") {
                e.preventDefault();
                onOpen(row);
              }
              if (e.key === " ") {
                e.preventDefault();
                choose(index, e.shiftKey, e.ctrlKey);
              }
              const step =
                e.key === "ArrowDown"
                  ? columns
                  : e.key === "ArrowUp"
                    ? -columns
                    : grid && e.key === "ArrowRight"
                      ? 1
                      : grid && e.key === "ArrowLeft"
                        ? -1
                        : 0;
              if (step) {
                e.preventDefault();
                const target = Math.max(
                  0,
                  Math.min(rows.length - 1, index + step),
                );
                choose(target, e.shiftKey, false);
                const sibling = e.currentTarget.parentElement?.querySelector(
                  `[data-index="${target}"]`,
                ) as HTMLElement | undefined;
                if (!sibling) {
                  const list = e.currentTarget.closest(
                    ".directory-list",
                  ) as HTMLElement;
                  scrollToRow(target);
                  requestAnimationFrame(() =>
                    list
                      .querySelector<HTMLElement>(`[data-index="${target}"]`)
                      ?.focus(),
                  );
                }
                sibling?.focus();
              }
            }}
          >
            {!grid && (
              <>
                <td className="result-size">
                  {row.kind === "folder"
                    ? "—"
                    : row.size == null
                      ? "未知"
                      : `${BigInt(row.size).toLocaleString()} B`}
                </td>
                <td>{fileTime(row.modified)}</td>
              </>
            )}
          </FileListItem>
        );
      }}
    >
      {!busy && !rows.length && (
        <p className="search-empty">此目录没有已索引的文件</p>
      )}
      {next !== null && (
        <button className="load-more" disabled={busy} onClick={onMore}>
          加载更多文件
        </button>
      )}
    </DetailsViewport>
  );
}
