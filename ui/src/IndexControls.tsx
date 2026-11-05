import { useEffect, useRef, useState } from "react";
import { indexRequest, type IndexStatus } from "./index-api";

export const indexLabels: Record<string, string> = {
  loading: "恢复索引中",
  building: "准备索引",
  not_indexed: "尚未索引",
  queued: "等待索引",
  enumerating: "读取 NTFS 记录",
  scanning: "读取文件元数据",
  replaying: "补齐扫描期间变更",
  ready: "索引已建立",
  snapshot: "扫描快照",
  failed: "索引未完成",
  syncing: "同步文件变更",
  needs_validation: "索引需要校验",
  waiting: "等待接入／恢复",
};
const running = new Set([
  "loading",
  "queued",
  "enumerating",
  "scanning",
  "replaying",
  "building",
  "syncing",
  "waiting",
]);
export default function IndexControls({
  partitionId,
  name,
  eligible,
  filesystem = "NTFS",
  onStatus,
}: {
  partitionId: string;
  name: string;
  eligible: boolean;
  filesystem?: string | null;
  onStatus?: (
    partitionId: string,
    status: IndexStatus | null,
    error: string,
  ) => void;
}) {
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [error, setError] = useState("");
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"journal" | "scan_only" | "create_journal">(
    "journal",
  );
  const [consent, setConsent] = useState(false);
  const [sending, setSending] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    onStatus?.(partitionId, status, error);
  }, [partitionId, status, error, onStatus]);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    let active = true;
    let timer: number;
    async function poll() {
      const reply = await indexRequest({
        index_state: { partition_id: partitionId },
      });
      if (!active) return;
      if (reply.kind !== "index") throw new Error("索引状态响应无效");
      setStatus(reply.data);
      setError("");
      timer = window.setTimeout(() => {
        void poll().catch(fail);
      }, 1500);
    }
    function fail(e: unknown) {
      if (!active) return;
      setError(String(e));
      timer = window.setTimeout(() => {
        void poll().catch(fail);
      }, 1500);
    }
    void poll().catch(fail);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [partitionId, revision]);
  useEffect(() => {
    if (!open) return;
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, [open]);
  const busy = !!status && running.has(status.phase);
  async function submit(cancel = false) {
    if (sending) return;
    if (cancel && !status?.task_id) return;
    setSending(true);
    const success = await indexRequest(
      cancel
        ? { task_cancel: { id: status!.task_id! } }
        : { index_start: { partition_id: partitionId, mode } },
    ).then(
      () => true,
      (e) => {
        setError(String(e));
        return false;
      },
    );
    if (success) {
      setOpen(false);
      setRevision((n) => n + 1);
    }
    setSending(false);
  }
  return (
    <div className="index-controls">
      <span>
        {error
          ? "索引状态不可用"
          : status
            ? indexLabels[status.phase] || status.phase
            : "正在读取索引"}
      </span>
      {status?.completed != null && !busy && status.phase !== "failed" && (
        <small>
          {new Date(status.completed * 1000).toLocaleString()} ·{" "}
          {status.records.toLocaleString()} 项
        </small>
      )}
      {status?.phase === "failed" && (
        <small>
          {status.active_generation != null
            ? "仍可搜索旧索引"
            : status.generation != null
              ? "仅可搜索已保存的未完成记录"
              : "尚无可搜索索引"}
        </small>
      )}
      {(error || status?.error) && (
        <small className="asset-error" role="alert">
          {error || status?.error}
        </small>
      )}
      {busy && status?.phase !== "loading" ? (
        <button
          disabled={sending || !status?.task_id}
          aria-label={`取消索引 ${name}`}
          onClick={() => void submit(true)}
        >
          取消索引
        </button>
      ) : (
        <button
          disabled={!eligible || busy || sending}
          aria-label={`索引 ${name}`}
          onClick={() => {
            setMode(filesystem === "NTFS" ? "journal" : "scan_only");
            setConsent(false);
            setOpen(true);
          }}
        >
          索引…
        </button>
      )}
      {open && (
        <dialog
          ref={dialog}
          className="asset-dialog index-dialog"
          aria-labelledby={`index-${partitionId}`}
          onCancel={(e) => {
            e.preventDefault();
            if (!sending) setOpen(false);
          }}
        >
          <h2 id={`index-${partitionId}`}>索引 {name}</h2>
          {error && (
            <p className="asset-error" role="alert">
              {error}
            </p>
          )}
          <p>重新读取这个分区。构建完成前，保留已有索引。</p>
          {filesystem === "NTFS" && (
            <label>
              <input
                type="radio"
                checked={mode === "journal"}
                onChange={() => setMode("journal")}
              />
              使用已有 USN 日志
            </label>
          )}
          <label>
            <input
              type="radio"
              checked={mode === "scan_only"}
              onChange={() => setMode("scan_only")}
            />
            基础扫描（不创建日志）
          </label>
          {mode === "scan_only" && (
            <p>在线监控变化后重新校验；重连需完整扫描，保留上次快照供搜索。</p>
          )}
          {filesystem === "NTFS" && (
            <label>
              <input
                type="radio"
                checked={mode === "create_journal"}
                onChange={() => setMode("create_journal")}
              />
              创建 USN 日志并索引
            </label>
          )}
          {mode === "create_journal" && (
            <>
              <p>
                会写入 NTFS 系统元数据，占用磁盘空间。新日志最大 32
                MiB，分配增量 8 MiB；已有日志保持原配置。
              </p>
              <label>
                <input
                  type="checkbox"
                  checked={consent}
                  onChange={(e) => setConsent(e.target.checked)}
                />
                允许在这个分区创建 USN 日志
              </label>
            </>
          )}
          <div>
            <button disabled={sending} onClick={() => setOpen(false)}>
              取消
            </button>
            <button
              disabled={
                sending || !eligible || (mode === "create_journal" && !consent)
              }
              onClick={() => void submit()}
            >
              开始索引
            </button>
          </div>
        </dialog>
      )}
    </div>
  );
}
