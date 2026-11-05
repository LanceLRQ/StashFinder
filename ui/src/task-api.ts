import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export type Task = {
  id: string;
  partition_id: string;
  kind: string;
  mode: "journal" | "create_journal" | "scan_only";
  automatic: boolean;
  state: string;
  created: number;
  updated: number;
  error: string | null;
  phase?: string | null;
};
export type TaskSnapshot = {
  tasks: Task[];
  automatic: boolean;
  paused: string[];
};
type TaskPage = {
  version: number;
  row: Task | null;
  next: number | null;
  automatic: boolean;
  paused: string[];
};
type TaskAction =
  | { task_cancel: { id: string } }
  | { task_retry: { id: string } }
  | { auto_update: { enabled: boolean } }
  | { partition_pause: { partition_id: string; paused: boolean } };

export async function taskCommand(action: TaskAction) {
  const reply = await invoke<{ kind: string; data?: string }>("index_request", {
    action,
  });
  if (reply.kind !== "accepted")
    throw new Error(reply.data || "任务操作响应无效");
}
export async function readTasks(): Promise<TaskSnapshot> {
  for (let attempt = 0; attempt < 3; attempt++) {
    let version: number | null = null;
    let offset = 0;
    const tasks: Task[] = [];
    let restart = false;
    do {
      const reply = await invoke<
        { kind: "tasks"; data: TaskPage } | { kind: "error"; data: string }
      >("index_request", { action: { task_page: { version, offset } } }).catch(
        (e: unknown) => {
          if (String(e).includes("任务快照已更新"))
            return { kind: "error" as const, data: String(e) };
          throw e;
        },
      );
      if (reply.kind === "error" && reply.data.includes("任务快照已更新")) {
        restart = true;
        break;
      }
      if (reply.kind !== "tasks")
        throw new Error(reply.kind === "error" ? reply.data : "任务响应无效");
      const page: TaskPage = reply.data;
      version = page.version;
      if (page.row) tasks.push(page.row);
      if (page.next === null)
        return { tasks, automatic: page.automatic, paused: page.paused };
      if (page.next <= offset || tasks.length > 4096)
        throw new Error("任务分页无效");
      offset = page.next;
    } while (!restart);
  }
  throw new Error("任务持续更新，请稍后重试");
}

export function useTasks(active: boolean) {
  const [data, setData] = useState<TaskSnapshot | null>(null);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((n) => n + 1), []);
  useEffect(() => {
    let alive = true;
    let timer: number;
    async function poll() {
      await readTasks().then(
        (next) => {
          if (alive) {
            setData(next);
            setError("");
          }
        },
        (e) => {
          if (alive) setError(String(e));
        },
      );
      if (alive)
        timer = window.setTimeout(
          () => void poll(),
          active && !document.hidden ? 2000 : 10000,
        );
    }
    void poll();
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [active, revision]);
  return { data, error, refresh };
}

export const taskLabels: Record<string, string> = {
  queued: "等待执行",
  running: "进行中",
  waiting: "等待接入／恢复",
  needs_confirmation: "等待确认",
  failed: "失败",
  interrupted: "已中断",
  completed: "已完成",
  cancelled: "已取消",
  superseded: "已重试",
};
export const taskKinds: Record<string, string> = {
  scan: "扫描校验",
  build: "建立索引",
  rebuild: "完整校验",
  sync: "增量更新",
};
export function taskGroup(task: Task) {
  return ["queued", "running", "waiting"].includes(task.state)
    ? "current"
    : ["failed", "interrupted", "needs_confirmation"].includes(task.state)
      ? "attention"
      : "history";
}
