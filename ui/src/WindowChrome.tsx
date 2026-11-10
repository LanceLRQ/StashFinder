import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export default function WindowChrome({ children }: { children: ReactNode }) {
  const desktop = "__TAURI_INTERNALS__" in window;
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    const current = getCurrentWindow();
    let active = true;
    const update = () => {
      void current.isMaximized().then((value) => {
        if (active) setMaximized(value);
      }, console.error);
    };
    const listening = current.onResized(update);
    void listening.then(update, console.error);
    return () => {
      active = false;
      void listening.then((unlisten) => unlisten(), console.error);
    };
  }, [desktop]);

  const maximizeLabel = maximized ? "还原" : "最大化";
  return (
    <header className="window-caption">
      {children}
      <div className="caption-drag" aria-hidden="true" />
      {desktop && (
        <div className="caption-controls" role="group" aria-label="窗口控件">
          <button
            aria-label="最小化"
            title="最小化"
            onClick={() =>
              void getCurrentWindow().minimize().catch(console.error)
            }
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="M1 6h10" />
            </svg>
          </button>
          <button
            aria-label={maximizeLabel}
            title={maximizeLabel}
            onClick={() => {
              const current = getCurrentWindow();
              void current
                .toggleMaximize()
                .then(() => current.isMaximized())
                .then(setMaximized, console.error);
            }}
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path
                d={
                  maximized
                    ? "M3.5 3.5V1.5h7v7h-2 M1.5 3.5h7v7h-7z"
                    : "M1.5 1.5h9v9h-9z"
                }
              />
            </svg>
          </button>
          <button
            className="caption-close"
            aria-label="关闭窗口"
            title="关闭"
            onClick={() => void getCurrentWindow().close().catch(console.error)}
          >
            <svg viewBox="0 0 12 12" aria-hidden="true">
              <path d="m2 2 8 8 M10 2 2 10" />
            </svg>
          </button>
        </div>
      )}
    </header>
  );
}
