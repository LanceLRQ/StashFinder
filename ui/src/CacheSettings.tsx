import {
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type Ref,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVisuals } from "./Visuals";
import Modal from "./Modal";
import {
  cacheBytes,
  type CacheProgress,
  type CacheStatus,
  type ThumbnailTypes,
} from "./visual-api";

const fileTypes = [
  ["images", "图片"],
  ["videos", "视频"],
  ["documents", "文档（PDF）"],
] as const;

export type CacheSettingsControls = { save: () => Promise<boolean> };

export default function CacheSettings({
  ref,
  onBusyChange,
}: {
  ref?: Ref<CacheSettingsControls>;
  onBusyChange?: (busy: boolean) => void;
}) {
  const visuals = useVisuals();
  const [status, setStatus] = useState<CacheStatus | null>(null);
  const [limit, setLimit] = useState("");
  const [unlimited, setUnlimited] = useState(false);
  const [types, setTypes] = useState<ThumbnailTypes | null>(null);
  const [extensions, setExtensions] = useState("");
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
          setLimit(
            next.limit_bytes === null ? "2" : String(next.limit_bytes / 1e9),
          );
          setUnlimited(next.limit_bytes === null);
          setTypes(next.thumbnail_types);
          setExtensions(next.thumbnail_types.extensions.join(", "));
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
  async function update(
    command:
      | "visual_cache_limit"
      | "visual_cache_clear"
      | "visual_thumbnail_types",
    args: Record<string, unknown> = {},
  ) {
    if (working.current) return false;
    const id = ++sequence.current;
    working.current = true;
    const requestId = crypto.randomUUID();
    operationId.current = requestId;
    setMaintenance(null);
    setBusy(true);
    onBusyChange?.(true);
    setError("");
    if (command === "visual_cache_clear") setConfirm(false);
    return invoke<CacheStatus>(
      command,
      command === "visual_thumbnail_types" ? args : { ...args, requestId },
    )
      .then(
        (next) => {
          if (!live.current || id !== sequence.current) return false;
          setStatus(next);
          if (command === "visual_cache_limit") {
            setUnlimited(next.limit_bytes === null);
            if (next.limit_bytes !== null)
              setLimit(String(next.limit_bytes / 1e9));
          }
          if (command === "visual_thumbnail_types") {
            setTypes(next.thumbnail_types);
            setExtensions(next.thumbnail_types.extensions.join(", "));
          }
          if (command === "visual_cache_clear") {
            setConfirm(false);
            visuals?.invalidate();
          }
          return true;
        },
        (failure) => {
          if (live.current && id === sequence.current)
            setError(String(failure).replace(/^Error: /, ""));
          return false;
        },
      )
      .finally(() => {
        working.current = false;
        if (live.current && id === sequence.current) {
          setBusy(false);
          onBusyChange?.(false);
        }
      });
  }
  useImperativeHandle(ref, () => ({
    async save() {
      if (!status || !types) return true;
      const limitBytes = unlimited ? null : Math.round(Number(limit) * 1e9);
      if (
        limitBytes !== null &&
        (!Number.isSafeInteger(limitBytes) || limitBytes <= 0)
      ) {
        setError("请输入有效的正数容量");
        return false;
      }
      const nextTypes = {
        ...types,
        extensions: extensions
          .split(/[,;，；\n]/)
          .map((ext) => ext.trim())
          .filter(Boolean),
      };
      if (
        limitBytes !== status.limit_bytes &&
        !(await update("visual_cache_limit", { limitBytes }))
      )
        return false;
      if (JSON.stringify(nextTypes) !== JSON.stringify(status.thumbnail_types))
        return update("visual_thumbnail_types", { types: nextTypes });
      return true;
    },
  }));
  return (
    <div className="cache-controls">
      {!status ? (
        <p role="status">{error ? "" : "读取缓存…"}</p>
      ) : (
        <>
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
                disabled={busy || unlimited}
                onChange={(event) => setLimit(event.target.value)}
              />
              <span>GB</span>
              <label>
                <input
                  type="checkbox"
                  checked={unlimited}
                  disabled={busy}
                  onChange={(event) => setUnlimited(event.target.checked)}
                />
                不限容量
              </label>
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
                {cacheBytes(status.used_bytes)} /{" "}
                {status.limit_bytes === null
                  ? "不限容量"
                  : cacheBytes(status.limit_bytes)}
              </span>
              {status.limit_bytes !== null && (
                <progress
                  aria-label="图片缓存占用"
                  value={status.used_bytes}
                  max={status.limit_bytes}
                />
              )}
              <span>
                {status.free_bytes === null
                  ? "磁盘剩余空间未知"
                  : `磁盘剩余 ${cacheBytes(status.free_bytes)}`}
              </span>
            </div>
            {(busy || status.waiting > 0 || status.active > 0) && (
              <p className="cache-activity" role="status">
                {busy
                  ? maintenance && maintenance.total > 0
                    ? `正在整理 ${maintenance.processed} / ${maintenance.total}`
                    : "正在更新缓存…"
                  : `等待 ${status.waiting} · 处理中 ${status.active}`}
              </p>
            )}
          </div>
          {types && (
            <fieldset className="thumbnail-types" disabled={busy}>
              <legend>预生成文件类型</legend>
              <div className="thumbnail-type-options">
                {fileTypes.map(([key, label]) => (
                  <label key={key}>
                    <input
                      type="checkbox"
                      checked={types[key]}
                      onChange={(event) =>
                        setTypes({ ...types, [key]: event.target.checked })
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
              <label htmlFor="thumbnail-extensions">自定义扩展名</label>
              <input
                id="thumbnail-extensions"
                type="text"
                value={extensions}
                maxLength={4096}
                placeholder="例如 jxl, docx"
                aria-describedby="thumbnail-extensions-help"
                onChange={(event) => setExtensions(event.target.value)}
              />
              <small id="thumbnail-extensions-help">
                用逗号分隔。生成结果取决于 Windows 对该格式的支持。
              </small>
            </fieldset>
          )}
        </>
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
        <Modal
          role="alertdialog"
          title="清空图片缓存？"
          onClose={() => setConfirm(false)}
          closeDisabled={busy}
          actions={
            <button
              disabled={busy}
              onClick={() => update("visual_cache_clear")}
            >
              确认清空
            </button>
          }
        >
          <p>保留文件索引、容量和预生成设置。</p>
          {error && (
            <p role="alert" className="desktop-error">
              {error}
            </p>
          )}
        </Modal>
      )}
    </div>
  );
}
