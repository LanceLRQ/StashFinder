import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Modal from "./Modal";

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
  offline: "等待分区重新连接",
  full: "缓存空间不足",
  cancelling: "正在取消",
  cancelled: "已取消",
  completed: "已完成",
  failed: "未完成",
};
type Job = {
  label: string;
  state: keyof typeof states;
  discovered: number;
  saved: number;
  skipped: number;
  unsupported: number;
  failed: number;
  error?: string;
};

export default function ThumbnailJobs({
  view,
  onProgress,
  onClose,
}: {
  view: ThumbnailScope | "progress";
  onProgress: () => void;
  onClose: () => void;
}) {
  const [rebuild, setRebuild] = useState(false);
  const [job, setJob] = useState<Job | null>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (view !== "progress") return;
    let live = true;
    let timer: number;
    async function load() {
      try {
        const status = await invoke<Job | null>("thumbnail_status");
        if (live) setJob(status);
      } catch (error) {
        if (live) setError(String(error));
      } finally {
        if (live) timer = window.setTimeout(load, 500);
      }
    }
    void load();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [view]);

  async function command(name: string, args: Record<string, unknown>) {
    setBusy(true);
    setError("");
    try {
      await invoke(name, args);
      if (name === "thumbnail_start") onProgress();
      else setJob(await invoke<Job | null>("thumbnail_status"));
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  }

  if (view !== "progress")
    return (
      <Modal
        title="预生成缩略图"
        onClose={onClose}
        closeDisabled={busy}
        actions={
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void command("thumbnail_start", { options: { ...view, rebuild } })
            }
          >
            开始
          </button>
        }
      >
        <p>
          {view.label}
          {view.folder ? "（含子文件夹）" : ""}
        </p>
        <label>
          处理方式{" "}
          <select
            value={rebuild ? "rebuild" : "skip"}
            onChange={(event) => setRebuild(event.target.value === "rebuild")}
            disabled={busy}
          >
            <option value="skip">跳过已有</option>
            <option value="rebuild">全部重建</option>
          </select>
        </label>
        <p>处理常见图片、视频和 PDF。</p>
        {error && <p role="alert">{error}</p>}
      </Modal>
    );

  const terminal =
    job &&
    ["completed", "cancelled", "failed", "cancelling"].includes(job.state);
  const paused = job && ["paused", "full"].includes(job.state);
  const processed =
    job && job.saved + job.skipped + job.unsupported + job.failed;
  return (
    <Modal
      title="缩略图任务"
      onClose={onClose}
      actions={
        job &&
        !terminal && (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                void command("thumbnail_control", {
                  action: paused ? "resume" : "pause",
                })
              }
            >
              {paused ? "继续" : "暂停"}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                void command("thumbnail_control", { action: "cancel" })
              }
            >
              取消任务
            </button>
          </>
        )
      }
    >
      {job ? (
        <>
          <p>
            {job.label} · {states[job.state]}
          </p>
          {job.state === "running" && <progress aria-label="正在生成缩略图" />}
          <p>
            已处理 {processed?.toLocaleString()} 项，已发现{" "}
            {job.discovered.toLocaleString()} 项
          </p>
          <p>
            已保存 {job.saved.toLocaleString()} · 已跳过{" "}
            {job.skipped.toLocaleString()} · 不支持{" "}
            {job.unsupported.toLocaleString()} · 失败{" "}
            {job.failed.toLocaleString()}
          </p>
          {job.error && <p>{job.error}</p>}
        </>
      ) : (
        <p>{job === null ? "暂无缩略图任务" : "正在读取任务…"}</p>
      )}
      {error && <p role="alert">{error}</p>}
    </Modal>
  );
}
