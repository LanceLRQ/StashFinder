import { useRef, useState } from "react";

export type Sample = {
  id: number;
  label: string;
  path: string;
  kind: string;
  available: boolean;
};
export type FileAction =
  | "copy_samples"
  | "drag_samples"
  | "cut_samples"
  | "refresh_samples"
  | "rename"
  | "recycle"
  | "delete";

export default function SampleList({
  samples,
  selected,
  disabled,
  onSelect,
  onTransfer,
}: {
  samples: Sample[];
  selected: number[];
  disabled: boolean;
  onSelect: (ids: number[]) => void;
  onTransfer: (action: FileAction, ids: number[]) => void;
}) {
  const anchor = useRef(0);
  const [cursor, setCursor] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const gesture = useRef<{
    x: number;
    y: number;
    ids: number[];
    collapse: boolean;
  } | null>(null);

  function select(index: number, ctrl: boolean, shift: boolean) {
    const id = samples[index].id;
    const range = samples
      .slice(
        Math.min(anchor.current, index),
        Math.max(anchor.current, index) + 1,
      )
      .map((sample) => sample.id);
    const ids = shift
      ? ctrl
        ? [...new Set([...selected, ...range])]
        : range
      : ctrl
        ? selected.includes(id)
          ? selected.filter((value) => value !== id)
          : [...selected, id]
        : [id];
    if (!shift) anchor.current = index;
    setCursor(index);
    onSelect(ids);
    return ids;
  }

  return (
    <div className="file-browser">
      <div className="file-columns" aria-hidden="true">
        <span>名称</span>
        <span>类型</span>
        <span>所在路径</span>
      </div>
      <div
        className="sample-list"
        role="listbox"
        aria-label="验证文件"
        aria-multiselectable="true"
        aria-disabled={disabled}
        aria-activedescendant={`sample-${samples[cursor].id}`}
        tabIndex={0}
        onBlur={(event) => {
          gesture.current = null;
          if (!event.currentTarget.contains(event.relatedTarget)) setMenu(null);
        }}
        onKeyDown={(event) => {
          if (disabled) return;
          if (event.key === "F5") {
            event.preventDefault();
            onTransfer("refresh_samples", []);
          } else if (event.key === "F2" || event.key === "Delete") {
            event.preventDefault();
            if (
              selected.length &&
              (event.key !== "F2" || selected.length === 1)
            )
              onTransfer(
                event.key === "F2"
                  ? "rename"
                  : event.shiftKey
                    ? "delete"
                    : "recycle",
                selected,
              );
          } else if (
            event.ctrlKey &&
            ["c", "x"].includes(event.key.toLowerCase())
          ) {
            event.preventDefault();
            if (selected.length)
              onTransfer(
                event.key.toLowerCase() === "c"
                  ? "copy_samples"
                  : "cut_samples",
                selected,
              );
          } else if (event.ctrlKey && event.key.toLowerCase() === "a") {
            event.preventDefault();
            onSelect(samples.map((sample) => sample.id));
          } else if (
            ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
          ) {
            event.preventDefault();
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? samples.length - 1
                  : Math.max(
                      0,
                      Math.min(
                        samples.length - 1,
                        cursor + (event.key === "ArrowDown" ? 1 : -1),
                      ),
                    );
            select(next, event.ctrlKey, event.shiftKey);
          } else if (event.key === " ") {
            event.preventDefault();
            select(cursor, event.ctrlKey, event.shiftKey);
          } else if (event.key === "Escape") {
            onSelect([]);
            setMenu(null);
            gesture.current = null;
          }
        }}
        onMouseMove={(event) => {
          const start = gesture.current;
          if (event.buttons !== 1) {
            gesture.current = null;
            return;
          }
          if (
            !start ||
            disabled ||
            Math.hypot(event.clientX - start.x, event.clientY - start.y) < 5
          )
            return;
          gesture.current = null;
          if (start.ids.length) onTransfer("drag_samples", start.ids);
        }}
        onDragStart={(event) => event.preventDefault()}
      >
        {samples.map((sample, index) => (
          <div
            id={`sample-${sample.id}`}
            role="option"
            aria-selected={selected.includes(sample.id)}
            className={`sample ${cursor === index ? "current" : ""}`}
            key={sample.id}
            onMouseDown={(event) => {
              if (event.button !== 0 || disabled) return;
              event.preventDefault();
              event.currentTarget.parentElement?.focus();
              setMenu(null);
              const preserve =
                !event.ctrlKey &&
                !event.shiftKey &&
                selected.includes(sample.id);
              const ids = preserve
                ? selected
                : select(index, event.ctrlKey, event.shiftKey);
              setCursor(index);
              gesture.current = {
                x: event.clientX,
                y: event.clientY,
                ids,
                collapse: preserve,
              };
            }}
            onMouseUp={() => {
              if (gesture.current?.collapse) select(index, false, false);
              gesture.current = null;
            }}
            onContextMenu={(event) => {
              event.preventDefault();
              if (disabled) return;
              event.currentTarget.parentElement?.focus();
              if (!selected.includes(sample.id)) select(index, false, false);
              setMenu({
                x: Math.min(event.clientX, window.innerWidth - 170),
                y: Math.min(event.clientY, window.innerHeight - 260),
              });
            }}
          >
            <span className="sample-name">
              {sample.kind === "目录" ? "▸ " : "· "}
              {sample.label}
              {sample.available === false && "（不可用）"}
            </span>
            <span className="kind">{sample.kind}</span>
            <span className="sample-path" title={sample.path}>
              {sample.path}
            </span>
          </div>
        ))}
        {menu && (
          <div className="file-menu" style={{ left: menu.x, top: menu.y }}>
            {(
              [
                ["copy_samples", "复制", "Ctrl+C"],
                ["cut_samples", "剪切", "Ctrl+X"],
                ["rename", "重命名", "F2"],
                ["recycle", "移入回收站", "Delete"],
                ["delete", "永久删除", "Shift+Del"],
                ["refresh_samples", "刷新", "F5"],
              ] as const
            ).map(([action, label, shortcut]) => (
              <button
                key={action}
                disabled={
                  disabled || (action === "rename" && selected.length !== 1)
                }
                onClick={() => {
                  setMenu(null);
                  onTransfer(action, selected);
                }}
              >
                {label} <span>{shortcut}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
