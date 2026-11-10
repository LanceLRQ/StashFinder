import { useEffect, useRef } from "react";
import {
  commandEnabled,
  contextCommands,
  fileCommands,
  type MenuAction,
} from "./file-operations";
export default function FileContextMenu({
  x,
  y,
  count,
  online,
  singleFile,
  sameParent,
  folder = false,
  paste = false,
  blocked = false,
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
  blocked?: boolean;
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
  const facts = {
    count,
    online,
    singleFile,
    sameParent,
    folder,
    paste,
    blocked,
  };
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
      {contextCommands
        .filter((action) => count > 0 || action === "paste")
        .map((action) => {
          const [label, key] = fileCommands[action];
          return (
            <button
              role="menuitem"
              key={action}
              disabled={!commandEnabled(action, facts)}
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
              {key && <kbd aria-hidden="true">{key}</kbd>}
            </button>
          );
        })}
    </div>
  );
}
