import { useEffect, useRef } from "react";
export type MenuAction =
  | "open"
  | "open_with"
  | "browse_parent"
  | "locate"
  | "cut"
  | "copy"
  | "copy_name"
  | "copy_path"
  | "rename"
  | "recycle"
  | "properties"
  | "more"
  | "paste";
export default function FileContextMenu({
  x,
  y,
  count,
  online,
  singleFile,
  sameParent,
  folder = false,
  paste = false,
  onAction,
  onClose,
}: {
  x: number;
  y: number;
  count: number;
  online: boolean;
  singleFile: boolean;
  sameParent: boolean;
  folder?: boolean;
  paste?: boolean;
  onAction: (action: MenuAction) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    (
      ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)") ??
      ref.current
    )?.focus();
    return () => {
      if (previous?.isConnected) previous.focus();
    };
  }, []);
  useEffect(() => {
    const outside = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [onClose]);
  const items: [MenuAction, string, string, boolean][] = [
    ["open", "打开", "Enter", count === 1 && (online || folder)],
    ["open_with", "打开方式...", "", online && singleFile],
    ["browse_parent", "在新标签页浏览所在目录", "", count === 1],
    ["locate", "在资源管理器显示", "", online && count === 1],
    ["cut", "剪切", "Ctrl+X", online && count > 0],
    ["copy", "复制文件", "Ctrl+C", online && count > 0],
    ["paste", "粘贴", "Ctrl+V", paste],
    ["copy_name", "复制名称", "", count > 0],
    ["copy_path", "复制路径", "", count > 0],
    ["rename", "重命名", "F2", online && count === 1],
    ["recycle", "删除", "Delete", online && count > 0],
    ["properties", "属性", "Alt+Enter", count > 0],
    ["more", "更多...", "", online && count > 0 && sameParent],
  ];
  return (
    <div
      ref={ref}
      role="menu"
      tabIndex={-1}
      className="file-context-menu formal-file-menu"
      style={{
        left: Math.max(0, Math.min(x, window.innerWidth - 270)),
        top: Math.max(0, Math.min(y, window.innerHeight - 460)),
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
          e.preventDefault();
          const buttons = [
            ...ref.current!.querySelectorAll<HTMLButtonElement>(
              "button:not(:disabled)",
            ),
          ];
          const current = buttons.indexOf(
            document.activeElement as HTMLButtonElement,
          );
          buttons[
            e.key === "Home"
              ? 0
              : e.key === "End"
                ? buttons.length - 1
                : (current + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) %
                  buttons.length
          ]?.focus();
        }
      }}
    >
      {items
        .filter(([action]) => count > 0 || action === "paste")
        .map(([action, label, key, enabled]) => (
          <button
            role="menuitem"
            key={action}
            disabled={!enabled}
            title={
              action === "more" && !sameParent
                ? "跨目录选择暂不支持 Windows 原生菜单"
                : undefined
            }
            onClick={() => {
              onAction(action);
              onClose();
            }}
          >
            <span>{label}</span>
            {key && <kbd>{key}</kbd>}
          </button>
        ))}
    </div>
  );
}
