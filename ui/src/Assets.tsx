import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import IndexControls from "./IndexControls";
import "./assets.css";

import type {
  Observation,
  Registration,
  Asset,
  Candidate,
  AssetSnapshot,
  AssetAction,
} from "./asset-api";
export type { AssetSnapshot } from "./asset-api";
function Modal({
  title,
  onCancel,
  children,
}: {
  title: string;
  onCancel: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return (
    <dialog
      ref={ref}
      className="asset-dialog"
      aria-labelledby={title}
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
    >
      {children}
    </dialog>
  );
}

function bytes(value: number | null | undefined) {
  if (value == null) return "未知";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })} ${units[unit]}`;
}
function date(value: number | null | undefined) {
  return value == null ? "未知" : new Date(value * 1000).toLocaleString();
}
function initialName(o: Observation) {
  return o.mounts[0]?.replace(/[\\/]$/, "") || "未命名分区";
}
const connectionLabels: Record<string, string> = {
  online: "在线",
  offline: "离线",
  unconfirmed: "未确认",
  uncertain: "身份待确认",
  changed: "文件系统已变化",
  historical: "历史记录",
  inaccessible: "不可访问",
};

export default function Assets({
  onSearch,
  onSnapshot,
  onIndexStatus,
  activity,
}: {
  onSearch?: (text: string) => void;
  onSnapshot?: (snapshot: AssetSnapshot) => void;
  onIndexStatus?: (
    partitionId: string,
    status: import("./index-api").IndexStatus | null,
    error: string,
  ) => void;
  activity?: ReactNode;
} = {}) {
  const [data, setData] = useState<AssetSnapshot | null>(null);
  const [message, setMessage] = useState("");
  const [working, setWorking] = useState(false);
  const [selected, setSelected] = useState<Record<string, string>>({});
  const [rename, setRename] = useState<Registration | null>(null);
  const [name, setName] = useState("");
  const [weakConfirm, setWeakConfirm] = useState(false);
  const [registerOpen, setRegisterOpen] = useState(false);
  const [filter, setFilter] = useState("all");
  const [query, setQuery] = useState("");
  useEffect(() => {
    if (data) onSnapshot?.(data);
  }, [data, onSnapshot]);
  const request = useRef(0);
  const mounted = useRef(true);
  const lock = useRef(false);

  const reload = useCallback(async () => {
    const sequence = ++request.current;
    const next = await invoke<AssetSnapshot>("asset_snapshot");
    if (mounted.current && sequence === request.current) setData(next);
    return next;
  }, []);

  useEffect(() => {
    mounted.current = true;
    void reload().catch((error) => {
      if (mounted.current) setMessage(String(error));
    });
    return () => {
      mounted.current = false;
      request.current++;
    };
  }, [reload]);

  useEffect(() => {
    if (!data || working) return;
    const timer = window.setTimeout(
      () => {
        void reload().catch((error) => setMessage(String(error)));
      },
      data.busy ? 750 : document.hidden ? 10000 : 5000,
    );
    return () => window.clearTimeout(timer);
  }, [data, working, reload]);

  async function waitForOperation() {
    const deadline = Date.now() + 30000;
    let next = await reload();
    while (next.busy && Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 250));
      next = await reload();
    }
    if (next.busy)
      throw new Error("设备响应超过 30 秒，请查看最新状态后重试。");
    if (next.error) throw new Error(next.error);
    return next;
  }

  async function command(action: AssetAction) {
    if (lock.current) return false;
    lock.current = true;
    setWorking(true);
    setMessage("");
    const success = await invoke("asset_command", { action })
      .then(waitForOperation)
      .then(
        () => true,
        (error) => {
          setMessage(String(error));
          return false;
        },
      );
    lock.current = false;
    setWorking(false);
    return success;
  }

  async function register(allowWeak = false) {
    if (!data || lock.current) return;
    if (
      !allowWeak &&
      data.candidates.some(
        (c) => c.weak && c.observation.candidate_id in selected,
      )
    ) {
      setWeakConfirm(true);
      return;
    }
    setWeakConfirm(false);
    lock.current = true;
    setWorking(true);
    setMessage("");
    let current = data;
    const errors: string[] = [];
    const completed: string[] = [];
    for (const [candidate_id, name] of Object.entries(selected)) {
      const candidate = current.candidates.find(
        (c) => c.observation.candidate_id === candidate_id,
      );
      const success = await invoke("asset_command", {
        action: {
          register: {
            version: current.version,
            candidate_id,
            name,
            allow_weak: !!candidate?.weak && allowWeak,
          },
        },
      })
        .then(waitForOperation)
        .then(
          (next) => {
            current = next;
            return true;
          },
          (error) => {
            errors.push(`${name}：${String(error)}`);
            return false;
          },
        );
      if (success) completed.push(candidate_id);
      else {
        const next = await reload().catch(() => null);
        if (next) current = next;
      }
    }
    setSelected((previous) =>
      Object.fromEntries(
        Object.entries(previous).filter(([id]) => !completed.includes(id)),
      ),
    );
    setMessage(errors.join("；"));
    lock.current = false;
    setWorking(false);
  }

  const disabled = working || !!data?.busy;
  const error = message || data?.error;
  const groups = new Map<string, Asset[]>();
  for (const asset of data?.assets || []) {
    const key = asset.registration.disk_id;
    groups.set(key, [...(groups.get(key) || []), asset]);
  }
  const candidateGroups = new Map<string, Candidate[]>();
  for (const candidate of data?.candidates || []) {
    const key =
      candidate.observation.disk_number?.toString() ??
      candidate.observation.candidate_id;
    candidateGroups.set(key, [...(candidateGroups.get(key) || []), candidate]);
  }

  return (
    <div className="assets-app">
      <div className="asset-toolbar">
        {onSearch && (
          <form
            className="overview-search"
            onSubmit={(e) => {
              e.preventDefault();
              onSearch(query);
            }}
          >
            <input
              type="search"
              aria-label="搜索所有文件"
              placeholder="搜索文件名或路径…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <button type="submit">搜索</button>
          </form>
        )}
        <select
          aria-label="连接筛选"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        >
          <option value="all">全部分区</option>
          <option value="online">在线</option>
          <option value="offline">离线</option>
          <option value="attention">需关注</option>
        </select>
        <button disabled={disabled} onClick={() => void command("refresh")}>
          刷新设备
        </button>
        <button
          disabled={!data || disabled}
          onClick={() => setRegisterOpen(true)}
        >
          登记分区
        </button>
        <div className="asset-stats" aria-label="已登记资产统计">
          <span>
            硬盘 <b>{data ? groups.size : "—"}</b>
          </span>
          <span
            aria-label={
              data ? `已登记 ${data.assets.length} 个分区` : "正在连接后台"
            }
          >
            分区 <b>{data ? data.assets.length : "—"}</b>
          </span>
          <span>
            在线{" "}
            <b>
              {data
                ? data.assets.filter((a) => a.connection === "online").length
                : "—"}
            </b>
          </span>
        </div>
      </div>
      {error && !registerOpen && (
        <p className="asset-error" role="alert">
          {error}
        </p>
      )}
      {disabled && <p role="status">正在处理设备信息…</p>}
      <div className="overview-body">
        <div className="disk-list">
          {data && !data.busy && !data.error && !data.assets.length && (
            <div className="asset-empty">
              <h2>尚未登记分区</h2>
              <p>选择“登记分区”后建立文件索引，离线时仍可搜索。</p>
            </div>
          )}
          {Array.from(groups, ([id, assets]) => (
            <section
              className="disk-group"
              key={id}
              hidden={
                !assets.some(
                  (a) =>
                    filter === "all" ||
                    (filter === "attention"
                      ? !["online", "offline"].includes(a.connection)
                      : a.connection === filter),
                )
              }
            >
              <h2>
                {assets[0].registration.observation.model || "物理硬盘"}
                <span>
                  {bytes(assets[0].registration.observation.disk_bytes)}
                </span>
              </h2>
              <div className="asset-table-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>分区名称</th>
                      <th>状态</th>
                      <th>挂载点</th>
                      <th>文件系统</th>
                      <th>总量 / 剩余</th>
                      <th>索引</th>
                      <th>
                        <span className="sr-only">操作</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {assets.map((asset) => {
                      const r = asset.registration;
                      const o = r.observation;
                      const capacity = o.capacity;
                      return (
                        <tr
                          key={r.partition_id}
                          hidden={
                            filter !== "all" &&
                            (filter === "attention"
                              ? ["online", "offline"].includes(asset.connection)
                              : asset.connection !== filter)
                          }
                        >
                          <td>
                            <strong>{r.name}</strong>
                            {asset.error && (
                              <small className="asset-error">
                                {asset.error}
                              </small>
                            )}
                          </td>
                          <td>
                            <span className={`connection ${asset.connection}`}>
                              {connectionLabels[asset.connection] || "未确认"}
                            </span>
                            <small>最后在线：{date(r.last_seen)}</small>
                          </td>
                          <td>{asset.mounts.join(" · ") || "—"}</td>
                          <td>{o.identity.filesystem || "未知"}</td>
                          <td>
                            {bytes(capacity?.total)} / {bytes(capacity?.free)}
                            {capacity && (
                              <small>
                                {asset.connection === "online"
                                  ? "采集于"
                                  : "上次记录"}{" "}
                                {date(capacity.sampled_at)} · 剩余{" "}
                                {capacity.total
                                  ? (
                                      (capacity.free / capacity.total) *
                                      100
                                    ).toFixed(1)
                                  : "—"}
                                %
                              </small>
                            )}
                          </td>
                          <td>
                            {r.state !== "preparing" ? (
                              <IndexControls
                                partitionId={r.partition_id}
                                onStatus={onIndexStatus}
                                name={r.name}
                                eligible={
                                  !disabled &&
                                  !asset.error &&
                                  asset.connection === "online" &&
                                  o.identity.filesystem === "NTFS"
                                }
                              />
                            ) : (
                              "初始化未完成"
                            )}
                          </td>
                          <td>
                            <button
                              disabled={disabled}
                              aria-label={`重命名 ${r.name}`}
                              onClick={() => {
                                setRename(r);
                                setName(r.name);
                              }}
                            >
                              改名
                            </button>
                            {r.state === "preparing" && (
                              <button
                                disabled={
                                  disabled || asset.connection !== "online"
                                }
                                onClick={() => {
                                  void command({
                                    retry: {
                                      version: data!.version,
                                      partition_id: r.partition_id,
                                      candidate_id: o.candidate_id,
                                    },
                                  });
                                }}
                              >
                                重试初始化
                              </button>
                            )}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </section>
          ))}
        </div>
        {activity && (
          <aside className="overview-activity" aria-label="索引动态">
            {activity}
          </aside>
        )}
      </div>
      {data && registerOpen && (
        <Modal
          title="candidate-title"
          onCancel={() => {
            if (!disabled) setRegisterOpen(false);
          }}
        >
          {error && (
            <p className="asset-error" role="alert">
              {error}
            </p>
          )}
          <button
            className="dialog-close"
            disabled={disabled}
            onClick={() => setRegisterOpen(false)}
          >
            关闭登记
          </button>
          <section
            className="candidate-section"
            aria-labelledby="candidate-title"
          >
            <div className="candidate-heading">
              <h2 id="candidate-title">选择分区登记</h2>
              <button
                disabled={
                  disabled ||
                  !Object.keys(selected).length ||
                  Object.values(selected).some(
                    (n) => !n.trim() || Array.from(n.trim()).length > 128,
                  )
                }
                onClick={() => {
                  void register();
                }}
              >
                登记所选分区
              </button>
            </div>
            {!data.candidates.length && !data.busy && !data.error && (
              <p>没有可显示的新设备。接入硬盘后刷新设备。</p>
            )}
            {Array.from(candidateGroups, ([disk, candidates]) => (
              <section className="candidate-disk" key={disk}>
                <h3>
                  磁盘 {candidates[0].observation.disk_number ?? "未知"} ·{" "}
                  {candidates[0].observation.model || "型号未知"}
                  <span>{bytes(candidates[0].observation.disk_bytes)}</span>
                </h3>
                {candidates.map((candidate) => {
                  const o = candidate.observation;
                  const id = o.candidate_id;
                  return (
                    <div
                      className={`candidate-row ${candidate.blocked ? "blocked" : ""}`}
                      key={id}
                    >
                      <label>
                        <input
                          type="checkbox"
                          disabled={disabled || !!candidate.blocked}
                          checked={id in selected}
                          onChange={(event) => {
                            const checked = event.target.checked;
                            setSelected((previous) =>
                              checked
                                ? { ...previous, [id]: initialName(o) }
                                : Object.fromEntries(
                                    Object.entries(previous).filter(
                                      ([key]) => key !== id,
                                    ),
                                  ),
                            );
                          }}
                        />
                        <span>
                          <strong>
                            {o.mounts.join(" · ") || "无盘符分区"}
                          </strong>
                          <small>
                            {o.identity.filesystem || "未知"} · 分区{" "}
                            {bytes(o.partition_bytes)}
                          </small>
                        </span>
                      </label>
                      <div>
                        {id in selected && (
                          <input
                            aria-label={`${initialName(o)} 自定义名称`}
                            disabled={disabled}
                            value={selected[id]}
                            onChange={(e) =>
                              setSelected((previous) => ({
                                ...previous,
                                [id]: e.target.value,
                              }))
                            }
                          />
                        )}
                        {candidate.blocked ? (
                          <small>{candidate.blocked}</small>
                        ) : (
                          candidate.weak && (
                            <small>身份信息不足，登记前需确认</small>
                          )
                        )}
                      </div>
                    </div>
                  );
                })}
              </section>
            ))}
          </section>
        </Modal>
      )}
      <footer className="asset-footer">
        仅扫描已登记分区。离线容量与文件记录保留最后一次快照。
      </footer>
      {rename && (
        <Modal
          title="rename-title"
          onCancel={() => {
            if (!disabled) setRename(null);
          }}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void command({
                rename: { partition_id: rename.partition_id, name },
              }).then((success) => {
                if (success) setRename(null);
              });
            }}
          >
            <h2 id="rename-title">修改分区名称</h2>
            <label>
              分区名称
              <input
                autoFocus
                value={name}
                disabled={disabled}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <p>仅修改软件内名称，不修改硬盘卷标。</p>
            {error && <p className="asset-error">{error}</p>}
            <div>
              <button
                type="button"
                disabled={disabled}
                onClick={() => setRename(null)}
              >
                取消
              </button>
              <button
                disabled={
                  disabled ||
                  !name.trim() ||
                  Array.from(name.trim()).length > 128
                }
              >
                保存名称
              </button>
            </div>
          </form>
        </Modal>
      )}
      {weakConfirm && (
        <Modal title="weak-title" onCancel={() => setWeakConfirm(false)}>
          <h2 id="weak-title">身份信息不足</h2>
          <p>
            这些分区可以按新分区登记，但再次接入时可能无法自动识别。不会自动关联已有记录。
          </p>
          <div>
            <button autoFocus onClick={() => setWeakConfirm(false)}>
              取消
            </button>
            <button
              onClick={() => {
                void register(true);
              }}
            >
              按新分区登记
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
