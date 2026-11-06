import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVisuals } from "./Visuals";
import { cacheBytes, type CacheProgress, type CacheStatus } from "./visual-api";

export default function CacheSettings() {
  const visuals = useVisuals();
  const [status, setStatus] = useState<CacheStatus | null>(null);
  const [limit, setLimit] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [maintenance, setMaintenance] = useState<CacheProgress | null>(null);
  const operationId = useRef("");
  const live = useRef(false);
  const working = useRef(false);
  const initialized = useRef(false);
  const sequence = useRef(0);
  const clearButton = useRef<HTMLButtonElement>(null);
  function refresh() {
    const id = ++sequence.current;
    void invoke<CacheStatus>("visual_cache_status").then(
      (next) => {
        if (!live.current || id !== sequence.current) return;
        setStatus(next);
        if (!initialized.current) {
          setLimit(String(next.limit_bytes / 1e9));
          initialized.current = true;
        }
      },
      (failure) => {
        if (live.current && id === sequence.current)
          setError(String(failure).replace(/^Error: /, ""));
      },
    );
  }
  useEffect(() => {
    live.current = true;
    let stop: (() => void) | undefined;
    void listen<{ request_id: string; progress: CacheProgress }>(
      "visual-cache-progress",
      (event) => {
        if (
          !live.current ||
          !working.current ||
          event.payload.request_id !== operationId.current
        )
          return;
        const progress = event.payload.progress;
        setMaintenance(progress);
        setStatus((previous) =>
          previous
            ? {
                ...previous,
                used_bytes: progress.used_bytes,
                limit_bytes: progress.limit_bytes,
              }
            : previous,
        );
      },
    ).then(
      (unlisten) => {
        if (live.current) stop = unlisten;
        else unlisten();
      },
      (failure) => {
        if (live.current) setError(String(failure).replace(/^Error: /, ""));
      },
    );
    refresh();
    const timer = window.setInterval(() => {
      if (!working.current) refresh();
    }, 1000);
    return () => {
      live.current = false;
      sequence.current++;
      clearInterval(timer);
      stop?.();
    };
  }, []);
  useEffect(() => {
    if (confirm) return () => clearButton.current?.focus();
  }, [confirm]);
  function update(command: "visual_cache_limit" | "visual_cache_clear") {
    if (working.current) return;
    const bytes = Math.round(Number(limit) * 1e9);
    if (
      command === "visual_cache_limit" &&
      (!Number.isSafeInteger(bytes) || bytes <= 0)
    ) {
      setError("请输入有效的正数容量");
      return;
    }
    const id = ++sequence.current;
    working.current = true;
    const requestId = crypto.randomUUID();
    operationId.current = requestId;
    setMaintenance(null);
    setBusy(true);
    setError("");
    if (command === "visual_cache_clear") setConfirm(false);
    void invoke<CacheStatus>(
      command,
      command === "visual_cache_limit"
        ? { limitBytes: bytes, requestId }
        : { requestId },
    )
      .then(
        (next) => {
          if (!live.current || id !== sequence.current) return;
          setStatus(next);
          setLimit(String(next.limit_bytes / 1e9));
          if (command === "visual_cache_clear") {
            setConfirm(false);
            visuals?.invalidate();
          }
        },
        (failure) => {
          if (live.current && id === sequence.current)
            setError(String(failure).replace(/^Error: /, ""));
        },
      )
      .finally(() => {
        working.current = false;
        if (live.current && id === sequence.current) setBusy(false);
      });
  }
  function dialogKey(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      setConfirm(false);
    }
    if (event.key === "Tab") {
      const buttons = [
        ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
          "button:not(:disabled)",
        ),
      ];
      if (event.shiftKey && document.activeElement === buttons[0]) {
        event.preventDefault();
        buttons.at(-1)?.focus();
      } else if (!event.shiftKey && document.activeElement === buttons.at(-1)) {
        event.preventDefault();
        buttons[0]?.focus();
      }
    }
  }
  return (
    <div className="cache-controls">
      {!status ? (
        <p role="status">{error ? "" : "读取缓存…"}</p>
      ) : (
        <div className="cache-toolbar">
          <div className="cache-limit-row">
            <label htmlFor="visual-cache-limit">容量上限</label>
            <input
              id="visual-cache-limit"
              aria-label="图片缓存上限（GB）"
              type="number"
              min="0.000000001"
              step="any"
              value={limit}
              disabled={busy}
              onChange={(event) => setLimit(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") update("visual_cache_limit");
              }}
            />
            <span>GB</span>
            <button
              disabled={busy}
              onClick={() => update("visual_cache_limit")}
            >
              应用
            </button>
            <button
              ref={clearButton}
              disabled={busy}
              onClick={() => {
                setError("");
                setConfirm(true);
              }}
            >
              清空缓存
            </button>
          </div>
          <div className="cache-usage-row">
            <span>
              {cacheBytes(status.used_bytes)} / {cacheBytes(status.limit_bytes)}
            </span>
            <progress
              aria-label="图片缓存占用"
              value={status.used_bytes}
              max={status.limit_bytes}
            />
          </div>
          {(busy || status.waiting > 0 || status.active > 0) && (
            <p className="cache-activity" role="status">
              {busy
                ? maintenance && maintenance.total > 0
                  ? `正在整理 ${maintenance.processed} / ${maintenance.total}`
                  : "正在整理缓存…"
                : `等待 ${status.waiting} · 处理中 ${status.active}`}
            </p>
          )}
        </div>
      )}
      {error && !confirm && (
        <p className="desktop-error" role="alert">
          {error}
        </p>
      )}
      {!status && error && (
        <button
          onClick={() => {
            setError("");
            refresh();
          }}
        >
          重试
        </button>
      )}
      {confirm && (
        <div className="operation-backdrop">
          <div
            className="operation-confirm"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="clear-cache-title"
            onKeyDown={dialogKey}
          >
            <h2 id="clear-cache-title">清空图片缓存？</h2>
            <p>保留文件索引和容量设置。</p>
            {error && (
              <p role="alert" className="desktop-error">
                {error}
              </p>
            )}
            <div>
              <button
                autoFocus
                disabled={busy}
                onClick={() => setConfirm(false)}
              >
                取消
              </button>
              <button
                disabled={busy}
                onClick={() => update("visual_cache_clear")}
              >
                确认清空
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
