import { useRef } from "react";
import { fileTime, type FileRow } from "./browse-api";
import DetailsViewport from "./DetailsViewport";
import RenameInput from "./RenameInput";
import FileIcon from "./FileIcon";
import { fileEditingKey, type EntryRef } from "./file-operations";
import { thumbnailLayouts, type FileView } from "./file-view";

const layout = { rowHeight: 34, overscan: 10 };
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
  const Item = grid ? "div" : "tr";
  const anchor = useRef(0);
  const dragging = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  const suppressClick = useRef(false);
  const selected = (row: FileRow) =>
    allSelected
      ? !selection.includes(row.entry.join(","))
      : selection.includes(row.entry.join(","));
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
      layout={layout}
      grid={grid}
      columns={columns}
      total={rows.length}
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
      row={(index, scrollToRow, columns) => {
        const row = rows[index];
        const name =
          editing?.entry === row.entry.join(",") ? (
            <RenameInput
              name={row.name}
              folder={row.kind === "folder"}
              onSubmit={editing.onSubmit}
              onCancel={editing.onCancel}
            />
          ) : (
            row.name
          );
        const icon = (
          <FileIcon
            active={active}
            name={row.name}
            folder={row.kind === "folder"}
            reference={context ? { ...context, entry: row.entry } : undefined}
            connection={connection}
            size={grid?.size}
            thumbnail={!!grid}
          />
        );
        return (
          <Item
            key={row.entry.join(",")}
            tabIndex={0}
            data-index={index}
            role={grid ? "option" : undefined}
            aria-rowindex={grid ? undefined : index + 2}
            aria-posinset={grid ? index + 1 : undefined}
            aria-setsize={grid ? rows.length : undefined}
            aria-selected={selected(row)}
            className={grid ? "thumbnail-cell" : undefined}
            title={grid ? row.name : undefined}
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
            {grid ? (
              <>
                {icon}
                {typeof name === "string" ? (
                  <span className="thumbnail-name">{name}</span>
                ) : (
                  name
                )}
              </>
            ) : (
              <>
                <td title={row.name}>
                  {icon}
                  {name}
                </td>
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
          </Item>
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
