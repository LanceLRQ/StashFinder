import { useEffect, useState, type ReactNode } from "react";
import { indexRequest, type IndexStatus } from "./index-api";
import Modal from "./Modal";

export const indexLabels: Record<string, string> = {
  loading: "恢复索引中",
  building: "准备索引",
  not_indexed: "尚未索引",
  queued: "等待索引",
  enumerating: "读取 NTFS 记录",
  scanning: "读取文件元数据",
  replaying: "补齐扫描期间变更",
  publishing: "校验并发布索引",
  ready: "索引已建立",
  snapshot: "扫描快照",
  failed: "索引未完成",
  syncing: "同步文件变更",
  needs_validation: "索引需要校验",
  needs_rebuild: "需要重建索引",
  waiting: "等待接入／恢复",
};
const indexing = new Set([
  "building",
  "enumerating",
  "scanning",
  "replaying",
  "publishing",
  "syncing",
]);
const running = new Set([...indexing, "loading", "queued", "waiting"]);
export default function IndexControls({
  partitionId,
  name,
  eligible,
  filesystem = "NTFS",
  onStatus,
  details,
  browse,
  managed = false,
}: {
  partitionId: string;
  name: string;
  eligible: boolean;
  filesystem?: string | null;
  details?: ReactNode;
  browse?: ReactNode;
  managed?: boolean;
  onStatus?: (
    partitionId: string,
    status: IndexStatus | null,
    error: string,
  ) => void;
}) {
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [error, setError] = useState("");
  const [open, setOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    onStatus?.(partitionId, status, error);
  }, [partitionId, status, error, onStatus]);
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
  const busy = !!status && running.has(status.phase);
  const journalMissing = filesystem === "NTFS" && !!status?.journal_missing;
  async function cancel() {
    if (sending || !status?.task_id) return;
    setSending(true);
    await indexRequest({ task_cancel: { id: status.task_id } }).then(
      () => setRevision((n) => n + 1),
      (e) => setError(String(e)),
    );
    setSending(false);
  }
  return (
    <>
      <dl>
        {details}
        <div>
          <dt>索引</dt>
          <dd>
            <span className="index-phase">
              {!error && status && indexing.has(status.phase) && (
                <span className="index-spinner" aria-hidden="true" />
              )}
              {error
                ? "索引状态不可用"
                : status
                  ? indexLabels[status.phase] || status.phase
                  : "正在读取索引"}
            </span>
            {status?.completed != null &&
              !busy &&
              status.phase !== "failed" && (
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
            {(error || journalMissing || status?.error) && (
              <small className="asset-error" role="alert">
                {error ||
                  (journalMissing ? "此分区没有 USN 日志" : status?.error)}
              </small>
            )}
          </dd>
        </div>
      </dl>
      <div className="asset-detail-actions">
        {browse}
        {managed ? null : busy && status?.phase !== "loading" ? (
          <button
            disabled={sending || !status?.task_id}
            aria-label={`取消索引 ${name}`}
            onClick={() => void cancel()}
          >
            取消索引
          </button>
        ) : (
          <button
            disabled={!eligible || busy || sending}
            aria-label={`索引设置 ${name}`}
            onClick={() => setOpen(true)}
          >
            索引设置
          </button>
        )}
      </div>
      {open && (
        <IndexModeDialog
          partitionId={partitionId}
          name={name}
          eligible={eligible && !busy}
          ntfs={filesystem === "NTFS"}
          journalMissing={journalMissing}
          rebuild={status?.phase === "needs_rebuild"}
          onClose={() => setOpen(false)}
          onStarted={() => {
            setOpen(false);
            setRevision((n) => n + 1);
          }}
        />
      )}
    </>
  );
}

export function IndexModeDialog({
  partitionId,
  name,
  eligible,
  ntfs,
  journalMissing,
  rebuild = false,
  onClose,
  onStarted,
}: {
  partitionId: string;
  name: string;
  eligible: boolean;
  ntfs: boolean;
  journalMissing: boolean;
  rebuild?: boolean;
  onClose: () => void;
  onStarted: () => void;
}) {
  const [choice, setMode] = useState<
    "journal" | "scan_only" | "create_journal"
  >(ntfs && !journalMissing ? "journal" : "scan_only");
  const mode = journalMissing && choice === "journal" ? "scan_only" : choice;
  const [consent, setConsent] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const title = rebuild ? "重建索引" : journalMissing ? "选择索引方式" : "索引";
  const description = rebuild
    ? "增量游标已失效。重建完成前，保留已有索引。"
    : journalMissing
      ? "此分区没有 USN 日志。构建完成前，保留已有索引。"
      : "重新读取这个分区。构建完成前，保留已有索引。";
  async function submit() {
    if (sending || !eligible || (mode === "create_journal" && !consent)) return;
    setSending(true);
    setError("");
    await indexRequest({
      index_start: { partition_id: partitionId, mode },
    }).then(onStarted, (e) => setError(String(e)));
    setSending(false);
  }
  return (
    <Modal
      className="asset-dialog index-dialog"
      title={`${title} ${name}`}
      onClose={onClose}
      closeDisabled={sending}
      actions={
        <button
          disabled={
            sending || !eligible || (mode === "create_journal" && !consent)
          }
          onClick={() => void submit()}
        >
          {rebuild ? "确认重建" : "开始索引"}
        </button>
      }
    >
      {error && (
        <p className="asset-error" role="alert">
          {error}
        </p>
      )}
      <p>{description}</p>
      {ntfs && !journalMissing && (
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
      {ntfs && (
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
            会写入 NTFS 系统元数据，占用磁盘空间。新日志最大 32 MiB，分配增量 8
            MiB；已有日志保持原配置。
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
    </Modal>
  );
}
