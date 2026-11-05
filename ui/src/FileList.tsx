import { useRef } from "react";
import { fileTime, type FileRow } from "./browse-api";

export default function FileList({
  rows,
  selection,
  onSelect,
  onOpen,
  next,
  busy,
  onMore,
}: {
  rows: FileRow[];
  selection: string[];
  onSelect: (ids: string[]) => void;
  onOpen: (row: FileRow) => void;
  next: number | null;
  busy: boolean;
  onMore: () => void;
}) {
  const anchor = useRef(0);
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
      );
    }
  }
  return (
    <div
      className="directory-list"
      aria-label="目录文件列表"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.ctrlKey && e.key.toLowerCase() === "a") {
          e.preventDefault();
          onSelect(rows.map((r) => r.entry.join(",")));
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
          {rows.map((row, index) => (
            <tr
              key={row.entry.join(",")}
              tabIndex={0}
              aria-selected={selection.includes(row.entry.join(","))}
              onClick={(e) => choose(index, e.shiftKey, e.ctrlKey)}
              onDoubleClick={() => onOpen(row)}
              onKeyDown={(e) => {
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
                  const sibling = e.currentTarget.parentElement?.children[
                    target
                  ] as HTMLElement | undefined;
                  sibling?.focus();
                }
              }}
            >
              <td title={row.name}>
                <span className={`file-mark ${row.kind}`} aria-hidden="true" />
                {row.name}
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
          ))}
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
