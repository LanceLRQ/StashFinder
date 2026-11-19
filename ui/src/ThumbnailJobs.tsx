import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Modal from "./Modal";
import WorkspaceIcon from "./WorkspaceIcon";

export type ThumbnailScope = {
  partitionId: string;
  filesystemId: string;
  label: string;
  folder?: number[];
  generation?: string;
};

const states = {
  running: "正在生成",
  paused: "已暂停",
  waiting_for_index: "等待同一硬盘的索引任务完成",
  offline: "等待分区重新连接",
  quota: "已达图片缓存上限",
  full: "缓存磁盘空间不足",
  cancelling: "正在取消",
  cancelled: "已取消",
  completed: "已完成",
  failed: "未完成",
};

type Job = {
  label: string;
  state: keyof typeof states;
  total: number | null;
  saved: number;
  skipped: number;
  analysing?: boolean;
  videos?: number;
  other?: number;
  unknown?: number;
  unsupported: number;
  failed: number;
  error?: string;
};

export default function ThumbnailJobs({
  scope,
  onStarted,
  onClose,
  onRecovery,
}: {
  scope: ThumbnailScope;
  onStarted: () => void;
  onClose: () => void;
  onRecovery: (target: "cache" | "tasks") => void;
}) {
  const [analyse, setAnalyse] = useState(false);
  const [thumbnails, setThumbnails] = useState(true);
  const [rebuild, setRebuild] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const recovery = error.includes("选择预生成的文件类型")
    ? "cache"
    : error.includes("已有缩略图任务在运行")
      ? "tasks"
      : null;
  async function start() {
    setBusy(true);
    setError("");
    try {
      await invoke("thumbnail_start", {
        options: {
          ...scope,
          rebuild,
          ...(analyse ? { analyse, thumbnails } : {}),
        },
      });
      onStarted();
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, "") || "缩略图任务未启动");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title="分析文件"
      className="thumbnail-start-dialog"
      onClose={onClose}
      closeDisabled={busy}
      actions={
        <>
          <button disabled={busy} onClick={onClose}>
            取消
          </button>
          <button
            className="thumbnail-start-button"
            disabled={busy || (!analyse && !thumbnails)}
            onClick={() => void start()}
          >
            开始
          </button>
        </>
      }
    >
      <div className="thumbnail-start">
        <div className="thumbnail-scope">
          <span className="thumbnail-scope-icon">
            <WorkspaceIcon name={scope.folder ? "folder" : "drive"} />
          </span>
          <div>
            <strong>{scope.label}</strong>
            {scope.folder && <small>含子文件夹</small>}
          </div>
        </div>
        <label>
          <input
            type="checkbox"
            checked={thumbnails}
            disabled={busy}
            onChange={(e) => setThumbnails(e.target.checked)}
          />
          生成缩略图
        </label>
        <label>
          <input
            type="checkbox"
            checked={analyse}
            disabled={busy}
            onChange={(e) => setAnalyse(e.target.checked)}
          />
          识别视频文件
        </label>
        <label>
          处理方式
          <select
            value={rebuild ? "rebuild" : "skip"}
            onChange={(event) => setRebuild(event.target.value === "rebuild")}
            disabled={busy}
          >
            <option value="skip">跳过已有</option>
            <option value="rebuild">全部重建</option>
          </select>
        </label>
        {error && <p role="alert">{error}</p>}
        {recovery && (
          <button onClick={() => onRecovery(recovery)}>
            {recovery === "cache" ? "选择文件类型" : "查看运行中的任务"}
          </button>
        )}
      </div>
    </Modal>
  );
}

export function ThumbnailProgress({ revision }: { revision: number }) {
  const [job, setJob] = useState<Job | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    let timer: number;
    async function load() {
      try {
        const status = await invoke<Job | null>("thumbnail_status");
        if (live) setJob(status);
      } catch {
        // Keep the last readable state during a transient connection failure.
      } finally {
        if (live) timer = window.setTimeout(load, 500);
      }
    }
    void load();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [revision]);

  async function control(action: "pause" | "resume" | "cancel") {
    setBusy(true);
    setFailed(false);
    try {
      await invoke("thumbnail_control", { action });
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  if (!job) return null;
  const total = job.total;
  const counting = total == null;
  const terminal = ["completed", "cancelled", "failed", "cancelling"].includes(
    job.state,
  );
  const paused = ["paused", "quota", "full"].includes(job.state);
  const skipped = job.skipped + job.unsupported;
  const processed = job.saved + skipped + job.failed;
  const status =
    counting && job.state === "running"
      ? "正在从索引统计待处理文件"
      : job.analysing && job.state === "running"
        ? "正在分析"
        : states[job.state];
  return (
    <section
      className="thumbnail-job"
      aria-label={job.analysing ? "文件分析任务" : "缩略图任务"}
    >
      <header>
        <div className="thumbnail-scope">
          <span className="thumbnail-scope-icon">
            <WorkspaceIcon name="folder" />
          </span>
          <div>
            <strong>{job.analysing ? "分析文件" : "预生成缩略图"}</strong>
            <small>{job.label}</small>
          </div>
        </div>
        {!terminal && (
          <div className="thumbnail-job-actions">
            <button
              disabled={busy}
              onClick={() => void control(paused ? "resume" : "pause")}
            >
              {paused ? "继续" : "暂停"}
            </button>
            <button disabled={busy} onClick={() => void control("cancel")}>
              取消任务
            </button>
          </div>
        )}
      </header>
      <div className="thumbnail-job-status">
        <span role="status">{status}</span>
        {total != null && (
          <span>
            {processed.toLocaleString()} / {total.toLocaleString()} 项
          </span>
        )}
      </div>
      {((counting && job.state === "running") ||
        (total != null && total > 0)) && (
        <progress
          aria-label={job.analysing ? "文件分析进度" : "缩略图处理进度"}
          max={total ?? 1}
          value={counting ? undefined : processed}
        />
      )}
      <dl className="thumbnail-job-counts">
        <div>
          <dt>成功</dt>
          <dd>{job.saved.toLocaleString()}</dd>
        </div>
        <div>
          <dt>失败</dt>
          <dd>{job.failed.toLocaleString()}</dd>
        </div>
        <div>
          <dt>跳过</dt>
          <dd>{skipped.toLocaleString()}</dd>
        </div>
      </dl>
      {job.analysing && (
        <p>
          视频 {job.videos} · 非视频 {job.other} · 待确认 {job.unknown}
        </p>
      )}
      {job.state === "failed" && job.error && <p role="alert">{job.error}</p>}
      {job.state === "quota" && <p>提高图片缓存上限或设为不限容量后继续。</p>}
      {job.state === "full" && <p>释放缓存磁盘空间后继续。</p>}
      {failed && <p role="alert">缩略图任务操作未完成</p>}
    </section>
  );
}
