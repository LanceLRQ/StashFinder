import { Fragment, useLayoutEffect, useRef, useState } from "react";
import { indexLabels } from "./IndexControls";
import {
  taskCommand,
  taskGroup,
  taskKinds,
  taskLabels,
  type TaskSnapshot,
} from "./task-api";

export default function Tasks({
  data,
  error,
  partitions,
  onRefresh,
  onChooseIndex,
  focusTask,
}: {
  data: TaskSnapshot | null;
  error: string;
  partitions: {
    id: string;
    name: string;
    connection: string;
    canChooseIndex: boolean;
  }[];
  onRefresh: () => void;
  onChooseIndex: (partitionId: string) => void;
  focusTask?: string;
}) {
  const initial = data?.tasks.find((t) => t.id === focusTask);
  const [group, setGroup] = useState(initial ? taskGroup(initial) : "recent");
  const [partition, setPartition] = useState("");
  const [expanded, setExpanded] = useState(focusTask || "");
  const [sending, setSending] = useState(false);
  const [actionError, setActionError] = useState("");
  const target = useRef<HTMLButtonElement>(null);
  const focused = useRef<string>(undefined);
  useLayoutEffect(() => {
    if (!focusTask) {
      focused.current = undefined;
      return;
    }
    if (!initial || focused.current === focusTask) return;
    const requestedGroup = taskGroup(initial);
    if (group !== requestedGroup || partition || expanded !== focusTask) {
      setGroup(requestedGroup);
      setPartition("");
      setExpanded(focusTask);
    } else if (target.current) {
      focused.current = focusTask;
      target.current.focus();
    }
  }, [focusTask, initial, group, partition, expanded]);
  const rows = (data?.tasks || []).filter(
    (t) => !partition || t.partition_id === partition,
  );
  const shown =
    group === "recent"
      ? rows.sort((a, b) => b.updated - a.updated).slice(0, 5)
      : rows.filter((task) => taskGroup(task) === group);
  async function command(action: Parameters<typeof taskCommand>[0]) {
    if (sending) return;
    setSending(true);
    setActionError("");
    await taskCommand(action).then(onRefresh, (e) => setActionError(String(e)));
    setSending(false);
  }
  return (
    <div className="task-workspace">
      <div className="task-toolbar">
        <div className="task-tabs" role="group" aria-label="任务分类">
          {[
            ["recent", "最近动态"],
            ["current", "当前"],
            ["attention", "需处理"],
            ["history", "历史"],
          ].map(([id, label]) => (
            <button
              key={id}
              aria-pressed={group === id}
              onClick={() => setGroup(id)}
            >
              {label}
              <span>
                {id === "recent"
                  ? Math.min(rows.length, 5)
                  : rows.filter((t) => taskGroup(t) === id).length}
              </span>
            </button>
          ))}
        </div>
        <select
          aria-label="任务分区"
          value={partition}
          onChange={(e) => setPartition(e.target.value)}
        >
          <option value="">全部分区</option>
          {partitions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <span className="task-policy">
          {error
            ? "状态未确认"
            : data
              ? `自动更新${data.automatic ? "开启" : "关闭"}`
              : "正在读取"}
        </span>
        <button onClick={onRefresh} disabled={sending}>
          刷新
        </button>
      </div>
      {(error || actionError) && (
        <p className="desktop-error" role="alert">
          {actionError || error}
        </p>
      )}
      {!!data?.paused.length && (
        <div className="paused-partitions">
          {data.paused.map((id) => (
            <span key={id}>
              {partitions.find((p) => p.id === id)?.name || id} · 已暂停{" "}
              <button
                disabled={sending || !!error}
                onClick={() =>
                  void command({
                    partition_pause: { partition_id: id, paused: false },
                  })
                }
              >
                继续更新
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="task-table-wrap">
        <table className="task-table">
          <thead>
            <tr>
              <th>分区</th>
              <th>任务</th>
              <th>状态</th>
              <th>更新时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((task) => {
              const part = partitions.find((p) => p.id === task.partition_id);
              const name = part?.name || task.partition_id;
              const online = part?.connection === "online";
              const current = taskGroup(task) === "current";
              return (
                <Fragment key={task.id}>
                  <tr>
                    <td>
                      <button
                        className="task-name"
                        ref={task.id === focusTask ? target : undefined}
                        aria-expanded={expanded === task.id}
                        onClick={() =>
                          setExpanded(expanded === task.id ? "" : task.id)
                        }
                      >
                        {name}
                      </button>
                      <small>
                        {online
                          ? "在线"
                          : part?.connection === "historical"
                            ? "历史记录"
                            : "未接入"}
                      </small>
                    </td>
                    <td>{taskKinds[task.kind] || task.kind}</td>
                    <td>
                      <span className={`task-state ${task.state}`}>
                        {task.phase
                          ? indexLabels[task.phase] || task.phase
                          : taskLabels[task.state] || task.state}
                      </span>
                      {(task.state === "needs_confirmation" || task.error) && (
                        <small className="task-error">
                          {task.state === "needs_confirmation"
                            ? "此分区没有 USN 日志"
                            : task.error}
                        </small>
                      )}
                    </td>
                    <td className="task-time">
                      {new Date(task.updated * 1000).toLocaleString()}
                    </td>
                    <td className="task-actions">
                      {current ? (
                        <>
                          <button
                            aria-label={`取消 ${name}`}
                            disabled={sending || !!error}
                            onClick={() =>
                              void command({ task_cancel: { id: task.id } })
                            }
                          >
                            取消
                          </button>
                          <button
                            aria-label={`暂停 ${name}`}
                            disabled={
                              sending ||
                              !!error ||
                              data?.paused.includes(task.partition_id)
                            }
                            onClick={() =>
                              void command({
                                partition_pause: {
                                  partition_id: task.partition_id,
                                  paused: true,
                                },
                              })
                            }
                          >
                            暂停
                          </button>
                        </>
                      ) : task.state === "needs_confirmation" ? (
                        <button
                          aria-label={`选择索引方式 ${name}`}
                          disabled={sending || !!error || !part?.canChooseIndex}
                          onClick={() => onChooseIndex(task.partition_id)}
                        >
                          选择索引方式…
                        </button>
                      ) : ["failed", "interrupted", "cancelled"].includes(
                          task.state,
                        ) ? (
                        <button
                          aria-label={`重试 ${name}`}
                          title={online ? "重新执行" : "接入并确认分区后可重试"}
                          disabled={sending || !!error || !online}
                          onClick={() =>
                            void command({ task_retry: { id: task.id } })
                          }
                        >
                          重试
                        </button>
                      ) : null}
                    </td>
                  </tr>
                  {expanded === task.id && (
                    <tr className="task-detail">
                      <td colSpan={5}>
                        <dl>
                          <div>
                            <dt>触发方式</dt>
                            <dd>
                              {task.automatic ? "后台自动更新" : "手动操作"}
                            </dd>
                          </div>
                          <div>
                            <dt>创建时间</dt>
                            <dd>
                              {new Date(task.created * 1000).toLocaleString()}
                            </dd>
                          </div>
                          <div>
                            <dt>任务 ID</dt>
                            <dd>{task.id}</dd>
                          </div>
                        </dl>
                        {task.state === "cancelled" && (
                          <p>保留已提交索引；继续更新需恢复这个分区。</p>
                        )}
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
            {!shown.length && (
              <tr>
                <td className="task-empty" colSpan={5}>
                  {data
                    ? group === "current" || group === "recent"
                      ? "当前没有任务"
                      : group === "attention"
                        ? "没有需要处理的任务"
                        : "暂无任务历史"
                    : "正在读取任务…"}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
