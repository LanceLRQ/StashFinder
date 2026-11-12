import { useState } from "react";
import { indexLabels } from "./IndexControls";
import {
  taskCommand,
  taskGroup,
  taskKinds,
  taskLabels,
  type TaskSnapshot,
} from "./task-api";

export default function IndexActivity({
  data,
  error,
  partitions,
  partition,
  onSelect,
  onRefresh,
  onChooseIndex,
}: {
  data: TaskSnapshot | null;
  error: string;
  partitions: {
    id: string;
    name: string;
    canRetry: boolean;
    canChooseIndex: boolean;
  }[];
  partition?: string;
  onSelect: (id: string) => void;
  onRefresh: () => void;
  onChooseIndex: (id: string) => void;
}) {
  const [sending, setSending] = useState(false);
  const [actionError, setActionError] = useState("");
  const names = new Map(partitions.map((item) => [item.id, item]));
  const inScope = (id: string) =>
    names.has(id) && (!partition || partition === id);
  const rows = (data?.tasks ?? []).filter((task) => inScope(task.partition_id));
  const current = rows.filter((task) => taskGroup(task) === "current");
  const activePartitions = new Set(current.map((task) => task.partition_id));
  const attention = rows.filter(
    (task) =>
      taskGroup(task) === "attention" &&
      !activePartitions.has(task.partition_id),
  );
  const paused = (data?.paused ?? []).filter(inScope);
  const recent = rows
    .filter((task) => taskGroup(task) === "history")
    .sort((a, b) => b.updated - a.updated);
  const disabled = sending || !!error;
  const indexedAt = partition ? data?.indexes[partition]?.indexed_at : null;

  async function command(action: Parameters<typeof taskCommand>[0]) {
    if (disabled) return;
    setSending(true);
    setActionError("");
    await taskCommand(action).then(onRefresh, (e) => setActionError(String(e)));
    setSending(false);
  }

  if (
    partition &&
    !(
      attention.length ||
      current.length ||
      paused.length ||
      recent.length ||
      error ||
      actionError
    )
  )
    return null;

  return (
    <section className="index-activity" aria-label="索引活动">
      {!partition && <h2>索引活动</h2>}
      {(actionError || error) && (
        <p className="asset-error" role="alert">
          {actionError || error}
        </p>
      )}
      {!partition &&
        !error &&
        !(attention.length || current.length || paused.length) && (
          <p className="index-activity-note">
            {data ? "暂无需要处理的索引任务" : "正在读取索引状态…"}
          </p>
        )}
      {partition &&
        indexedAt != null &&
        (current.length > 0 || attention.length > 0) && (
          <p className="index-activity-note">
            最近成功更新 · {new Date(indexedAt * 1000).toLocaleString()}
          </p>
        )}
      {[
        { title: "需要处理", tasks: attention, active: false },
        { title: "正在更新", tasks: current, active: true },
      ].map(
        ({ title, tasks, active }) =>
          tasks.length > 0 && (
            <section key={title}>
              <h3>{title}</h3>
              <ul>
                {tasks.map((task) => {
                  const item = names.get(task.partition_id)!;
                  const state = taskLabels[task.state] || task.state;
                  const phase =
                    (task.phase && indexLabels[task.phase]) || state;
                  const label = partition
                    ? active
                      ? taskKinds[task.kind] || task.kind
                      : state
                    : `${item.name} · ${phase}`;
                  return (
                    <li key={task.id}>
                      <div className="index-activity-row">
                        {!partition && task.state === "running" && !error && (
                          <span className="index-spinner" aria-hidden="true" />
                        )}
                        {partition ? (
                          <span className="index-activity-label" title={label}>
                            {label}
                          </span>
                        ) : (
                          <button
                            className="index-activity-label"
                            title={
                              task.error ? `${label} · ${task.error}` : label
                            }
                            onClick={() => onSelect(item.id)}
                          >
                            {label}
                          </button>
                        )}
                        {active ? (
                          <>
                            {!paused.includes(item.id) && (
                              <button
                                aria-label={`暂停 ${item.name}`}
                                disabled={disabled}
                                onClick={() =>
                                  void command({
                                    partition_pause: {
                                      partition_id: item.id,
                                      paused: true,
                                    },
                                  })
                                }
                              >
                                暂停
                              </button>
                            )}
                            <button
                              aria-label={`取消 ${item.name}`}
                              disabled={disabled}
                              onClick={() =>
                                void command({ task_cancel: { id: task.id } })
                              }
                            >
                              取消
                            </button>
                          </>
                        ) : task.state === "needs_confirmation" ? (
                          <button
                            aria-label={`选择索引方式 ${item.name}`}
                            disabled={disabled || !item.canChooseIndex}
                            onClick={() => onChooseIndex(item.id)}
                          >
                            选择索引方式
                          </button>
                        ) : (
                          <button
                            aria-label={`重试 ${item.name}`}
                            disabled={disabled || !item.canRetry}
                            onClick={() =>
                              void command({ task_retry: { id: task.id } })
                            }
                          >
                            重试
                          </button>
                        )}
                      </div>
                      {partition && task.error && (
                        <p className="index-activity-note">{task.error}</p>
                      )}
                    </li>
                  );
                })}
              </ul>
            </section>
          ),
      )}
      {paused.length > 0 && (
        <section>
          <h3>已暂停</h3>
          <ul>
            {paused.map((id) => {
              const name = names.get(id)!.name;
              return (
                <li key={id} className="index-activity-row">
                  <span className="index-activity-label" title={name}>
                    {name}
                  </span>
                  <button
                    aria-label={`继续更新 ${name}`}
                    disabled={disabled}
                    onClick={() =>
                      void command({
                        partition_pause: { partition_id: id, paused: false },
                      })
                    }
                  >
                    继续更新
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {recent.length > 0 && (
        <details>
          <summary>最近操作</summary>
          <ul>
            {recent.map((task) => {
              const label = [
                ...(!partition ? [names.get(task.partition_id)!.name] : []),
                taskKinds[task.kind] || task.kind,
                taskLabels[task.state] || task.state,
                new Date(task.updated * 1000).toLocaleString(),
              ].join(" · ");
              return (
                <li
                  key={task.id}
                  className="index-activity-history"
                  title={task.error ? `${label} · ${task.error}` : label}
                >
                  {label}
                </li>
              );
            })}
          </ul>
        </details>
      )}
    </section>
  );
}
