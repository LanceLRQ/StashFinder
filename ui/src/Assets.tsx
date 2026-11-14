import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import IndexControls from "./IndexControls";
import { FileStatusBar } from "./FileOperations";
import Modal from "./Modal";
import WorkspaceIcon from "./WorkspaceIcon";
import "./assets.css";

import type {
  Observation,
  Candidate,
  Registration,
  Asset,
  AssetSnapshot,
  AssetAction,
} from "./asset-api";
export type { AssetSnapshot } from "./asset-api";

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
  return Array.from(
    o.volume_label?.trim() ||
      o.mounts[0]?.replace(/[\\/]$/, "") ||
      "未命名分区",
  )
    .slice(0, 128)
    .join("");
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

export type AssetControls = {
  register: () => void;
  refresh: () => void;
  retry: () => void;
};

export default function Assets({
  onSearch,
  onBrowse,
  onSnapshot,
  onIndexStatus,
  ref,
  onStatus,
  indexActivity,
  attentionPartitions,
  indexActivityLabels,
  managedIndexTasks,
}: {
  onSearch?: (text: string) => void;
  onBrowse?: (partition: string) => void;
  onSnapshot?: (snapshot: AssetSnapshot) => void;
  onIndexStatus?: (
    partitionId: string,
    status: import("./index-api").IndexStatus | null,
    error: string,
  ) => void;
  ref?: Ref<AssetControls>;
  onStatus?: (status: { error: string; working: boolean }) => void;
  indexActivity?: (
    partition: string | undefined,
    select: (id: string) => void,
  ) => ReactNode;
  attentionPartitions?: ReadonlySet<string>;
  indexActivityLabels?: ReadonlyMap<string, string>;
  managedIndexTasks?: ReadonlySet<string>;
} = {}) {
  const [data, setData] = useState<AssetSnapshot | null>(null);
  const [message, setMessage] = useState("");
  const [working, setWorking] = useState(false);
  const [selected, setSelected] = useState<
    Record<string, { candidate: Candidate; name: string }>
  >({});
  const [rename, setRename] = useState<Registration | null>(null);
  const [removing, setRemoving] = useState<Registration | null>(null);
  const [name, setName] = useState("");
  const [weakConfirm, setWeakConfirm] = useState(false);
  const [registerOpen, setRegisterOpen] = useState(false);
  const [autoIndex, setAutoIndex] = useState(true);
  const [grouped, setGrouped] = useState(false);
  const [selectedPartition, setSelectedPartition] = useState("");
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
    if (!data || data.busy || lock.current || !selectedCandidates.length)
      return;
    if (!allowWeak && selectedCandidates.some((c) => c.weak)) {
      setRegisterOpen(false);
      setWeakConfirm(true);
      return;
    }
    setWeakConfirm(false);
    closeRegistration();
    lock.current = true;
    setWorking(true);
    setMessage("");
    let current = data;
    const errors: string[] = [];
    const completed: string[] = [];
    function sameCandidate(candidate: Candidate, next: AssetSnapshot) {
      const found = next.candidates.find(
        (row) =>
          row.observation.candidate_id === candidate.observation.candidate_id,
      );
      return (
        found &&
        !found.blocked &&
        !found.registered &&
        found.weak === candidate.weak &&
        JSON.stringify(found.observation.identity) ===
          JSON.stringify(candidate.observation.identity)
      );
    }
    for (const candidate of selectedCandidates) {
      const candidate_id = candidate.observation.candidate_id;
      const { candidate: chosen, name } = selected[candidate_id];
      const submit = async () => {
        if (!sameCandidate(chosen, current)) {
          completed.push(candidate_id);
          throw new Error("设备身份已变化，请重新选择");
        }
        return invoke("asset_command", {
          action: {
            register: {
              version: current.version,
              candidate_id,
              name,
              allow_weak: candidate.weak && allowWeak,
              auto_index: autoIndex,
            },
          },
        });
      };
      const success = await submit()
        .catch(async (error) => {
          if (String(error) !== "设备快照已更新，请刷新") throw error;
          current = await waitForOperation();
          if (chosen.weak) {
            completed.push(candidate_id);
            throw new Error("设备身份信息不足，请重新选择");
          }
          return submit();
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
  useEffect(() => {
    onStatus?.({ error: error || "", working });
  }, [error, working, onStatus]);
  const registrationTrigger = useRef<HTMLElement | null>(null);
  const renameInput = useRef<HTMLInputElement>(null);
  function openRegistration() {
    if (!data || data.busy || lock.current) return;
    registrationTrigger.current = document.activeElement as HTMLElement;
    setAutoIndex(true);
    setRegisterOpen(true);
  }
  function closeRegistration() {
    setRegisterOpen(false);
    const trigger = registrationTrigger.current;
    window.setTimeout(() => trigger?.focus(), 0);
  }
  useImperativeHandle(ref, () => ({
    register: openRegistration,
    refresh: () => {
      if (!disabled) void command("refresh");
    },
    retry: () => {
      setMessage("");
      void reload().catch((error) => setMessage(String(error)));
    },
  }));
  const groups = new Map<string, Asset[]>();
  for (const asset of data?.assets || []) {
    const key = asset.registration.disk_id;
    groups.set(key, [...(groups.get(key) || []), asset]);
  }
  const candidates = (data?.candidates ?? []).filter(
    (candidate) => candidate.registered || !candidate.blocked,
  );
  const selectedCandidates = candidates.filter(
    (candidate) =>
      !candidate.registered && candidate.observation.candidate_id in selected,
  );
  const current = data?.assets.find(
    (asset) => asset.registration.partition_id === selectedPartition,
  );
  const observation = current?.registration.observation;
  const capacity = observation?.capacity;
  const capacityDeferred =
    current?.capacity_error === "索引写入中，容量保留上次快照";
  const sections = grouped
    ? Array.from(groups, ([key, assets]) => ({
        key,
        title: assets[0].registration.observation.model || "型号未知",
        assets,
      }))
    : [{ key: "all", title: "", assets: data?.assets ?? [] }];

  const details = current && observation && (
    <>
      <div>
        <dt>磁盘型号</dt>
        <dd>{observation.model || "未知"}</dd>
      </div>
      <div>
        <dt>卷名</dt>
        <dd>{observation.volume_label || "无卷名"}</dd>
      </div>
      <div>
        <dt>挂载点</dt>
        <dd>{current.mounts.join("、") || "未挂载"}</dd>
      </div>
      <div>
        <dt>文件系统</dt>
        <dd>{observation.identity.filesystem || "未知"}</dd>
      </div>
      <div>
        <dt>状态</dt>
        <dd>{connectionLabels[current.connection] || "未知"}</dd>
      </div>
      <div>
        <dt>容量</dt>
        <dd>{bytes(capacity?.total ?? observation.partition_bytes)}</dd>
      </div>
      <div>
        <dt>可用空间</dt>
        <dd>{bytes(capacity?.free)}</dd>
      </div>
      {capacity && (
        <div>
          <dt>已用</dt>
          <dd>{bytes(capacity.total - capacity.free)}</dd>
        </div>
      )}
      {capacity && (
        <div>
          <dt>
            {current.connection === "online" && !current.capacity_error
              ? "采集时间"
              : "上次记录"}
          </dt>
          <dd>{date(capacity.sampled_at)}</dd>
        </div>
      )}
      {current.connection !== "online" && (
        <div>
          <dt>最后在线</dt>
          <dd>{date(current.registration.last_seen)}</dd>
        </div>
      )}
    </>
  );
  const browse = current && onBrowse && (
    <button onClick={() => onBrowse(current.registration.partition_id)}>
      浏览文件
    </button>
  );

  return (
    <div className="assets-app">
      <div className="asset-toolbar">
        {onSearch && (
          <form
            className="overview-search"
            onSubmit={(event) => {
              event.preventDefault();
              const text = query.trim();
              if (text) onSearch(text);
            }}
          >
            <WorkspaceIcon name="search" className="search-field-icon" />
            <input
              type="search"
              aria-label="搜索所有文件"
              aria-description="按 Enter 搜索"
              placeholder="搜索文件名或路径…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <kbd aria-hidden="true">Enter</kbd>
          </form>
        )}
        <button
          type="button"
          aria-pressed={grouped}
          onClick={() => setGrouped((value) => !value)}
        >
          按硬盘分组
        </button>
        {!!data?.assets.length && (
          <button disabled={disabled} onClick={openRegistration}>
            登记分区…
          </button>
        )}
      </div>
      <div className="overview-body">
        <div
          className="disk-list"
          onClick={(event) => {
            if (event.target === event.currentTarget) setSelectedPartition("");
          }}
        >
          {!data && !error && <p role="status">正在读取已登记分区…</p>}
          {data && !data.assets.length && !data.busy && !error && (
            <div className="asset-empty">
              <WorkspaceIcon name="drive" />
              <h2>尚未登记分区</h2>
              <button disabled={disabled} onClick={openRegistration}>
                登记分区…
              </button>
            </div>
          )}
          {sections.map(
            ({ key, title, assets }) =>
              assets.length > 0 && (
                <section className="disk-group" key={key}>
                  {title && <h2>{title}</h2>}
                  <div className="disk-grid">
                    {assets.map((asset) => {
                      const row = asset.registration;
                      const indexActivityLabel = indexActivityLabels?.get(
                        row.partition_id,
                      );
                      const needsAttention =
                        !indexActivityLabel &&
                        attentionPartitions?.has(row.partition_id);
                      const info = row.observation;
                      const space = info.capacity;
                      const mounts = asset.mounts.length
                        ? asset.mounts
                        : info.mounts;
                      return (
                        <button
                          key={row.partition_id}
                          className="disk-card"
                          aria-label={row.name}
                          aria-description={
                            indexActivityLabel ||
                            (needsAttention ? "索引需要处理" : undefined)
                          }
                          aria-pressed={selectedPartition === row.partition_id}
                          onClick={() => setSelectedPartition(row.partition_id)}
                          onDoubleClick={() => onBrowse?.(row.partition_id)}
                          onKeyDown={(event) => {
                            if (
                              event.key === "Enter" &&
                              selectedPartition === row.partition_id &&
                              onBrowse
                            ) {
                              event.preventDefault();
                              onBrowse(row.partition_id);
                            }
                          }}
                        >
                          <WorkspaceIcon name="drive" />
                          <span className="disk-card-content">
                            <span className="disk-card-title">
                              <strong title={row.name}>{row.name}</strong>
                              {indexActivityLabel ? (
                                <span className="disk-index-active">
                                  {indexActivityLabel}
                                </span>
                              ) : (
                                needsAttention && (
                                  <span className="disk-index-attention">
                                    待处理
                                  </span>
                                )
                              )}
                            </span>
                            <span className="disk-card-meta">
                              {mounts[0]?.replace(/[\\/]$/, "") || "无盘符"} ·{" "}
                              {info.identity.filesystem || "未知"}
                              <span
                                className={`connection ${asset.connection}`}
                              >
                                {connectionLabels[asset.connection] || "未知"}
                              </span>
                            </span>
                            {!!space?.total && (
                              <meter
                                aria-label="已用容量"
                                min={0}
                                max={space.total}
                                value={space.total - space.free}
                              />
                            )}
                            <small>
                              {space
                                ? `${bytes(space.free)} 可用 / ${bytes(space.total)}`
                                : `${bytes(info.partition_bytes)} · 可用空间未知`}
                            </small>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                </section>
              ),
          )}
        </div>
        <aside className="asset-details" aria-label="分区详情">
          {current && observation && (
            <>
              <div className="asset-detail-title">
                <h2>{current.registration.name}</h2>
                <button
                  className="asset-rename"
                  disabled={disabled}
                  aria-label={`重命名 ${current.registration.name}`}
                  title="重命名"
                  onClick={() => {
                    setRename(current.registration);
                    setName(current.registration.name);
                  }}
                >
                  <WorkspaceIcon name="pencil" />
                </button>
              </div>
              {current.registration.state === "preparing" ? (
                <>
                  <dl>{details}</dl>
                  <div className="asset-detail-actions">
                    {browse}
                    <button
                      disabled={disabled || current.connection !== "online"}
                      onClick={() => {
                        void command({
                          retry: {
                            version: data!.version,
                            partition_id: current.registration.partition_id,
                            candidate_id: observation.candidate_id,
                          },
                        });
                      }}
                    >
                      重试初始化
                    </button>
                  </div>
                </>
              ) : (
                <IndexControls
                  key={current.registration.partition_id}
                  partitionId={current.registration.partition_id}
                  onStatus={onIndexStatus}
                  name={current.registration.name}
                  filesystem={observation.identity.filesystem}
                  details={details}
                  browse={browse}
                  managed={managedIndexTasks?.has(
                    current.registration.partition_id,
                  )}
                  eligible={
                    !disabled &&
                    !current.error &&
                    current.connection === "online" &&
                    current.registration.state === "ready"
                  }
                />
              )}
              {current.error && <p className="asset-error">{current.error}</p>}
              {current.capacity_error && !capacityDeferred && (
                <p className="asset-error">{current.capacity_error}</p>
              )}
            </>
          )}
          {indexActivity?.(
            current?.registration.partition_id,
            setSelectedPartition,
          )}
          {current && (
            <div className="asset-remove">
              <button
                disabled={disabled}
                onClick={() => setRemoving(current.registration)}
              >
                移除登记…
              </button>
            </div>
          )}
        </aside>
      </div>
      {data && registerOpen && (
        <Modal
          title="选择分区登记"
          className="asset-dialog registration-dialog"
          onClose={closeRegistration}
          actions={
            <>
              <label className="registration-auto-index">
                <input
                  type="checkbox"
                  checked={autoIndex}
                  disabled={disabled}
                  onChange={(event) => setAutoIndex(event.target.checked)}
                />
                登记后自动建立索引
              </label>
              <span className="registration-count" aria-live="polite">
                {selectedCandidates.length > 0 &&
                  `已选 ${selectedCandidates.length} 个分区`}
              </span>
              <button
                className="registration-submit"
                disabled={disabled || !selectedCandidates.length}
                onClick={() => void register()}
              >
                登记所选分区
              </button>
            </>
          }
        >
          {error && (
            <p className="asset-error" role="alert">
              {error}
            </p>
          )}
          <div className="candidate-section">
            {data.busy && !candidates.length && !error && (
              <p role="status">正在读取分区…</p>
            )}
            {!candidates.length && !data.busy && !error && (
              <p>当前没有可登记的分区。</p>
            )}
            <div className="candidate-grid">
              {candidates.map((candidate) => {
                const item = candidate.observation;
                const id = item.candidate_id;
                return (
                  <label key={id} className="candidate-card">
                    <input
                      type="checkbox"
                      disabled={disabled || !!candidate.registered}
                      checked={!candidate.registered && id in selected}
                      onChange={(event) => {
                        const checked = event.target.checked;
                        setSelected((previous) => {
                          const next = { ...previous };
                          if (checked)
                            next[id] = { candidate, name: initialName(item) };
                          else delete next[id];
                          return next;
                        });
                      }}
                    />
                    <WorkspaceIcon name="drive" />
                    <span className="candidate-copy">
                      <span className="candidate-title">
                        <strong>
                          {item.mounts[0]?.replace(/[\\/]$/, "") || "无盘符"}
                        </strong>
                        {item.volume_label && (
                          <span title={item.volume_label}>
                            {item.volume_label}
                          </span>
                        )}
                      </span>
                      <small>
                        {item.identity.filesystem || "未知"} ·{" "}
                        {bytes(item.partition_bytes ?? item.capacity?.total)}
                      </small>
                    </span>
                    {candidate.registered && (
                      <small className="candidate-status">已登记</small>
                    )}
                  </label>
                );
              })}
            </div>
          </div>
        </Modal>
      )}
      <FileStatusBar
        className="asset-footer"
        liveStatus={
          working
            ? "正在处理设备信息…"
            : capacityDeferred
              ? (current?.capacity_error ?? null)
              : null
        }
        essential={
          error && !registerOpen && !rename && !removing ? (
            <span className="asset-error" role="alert" title={error}>
              {error}
            </span>
          ) : null
        }
      >
        <div className="asset-stats" aria-label="已登记资产统计">
          <span>硬盘 {data ? groups.size : "—"}</span>
          <span
            aria-label={
              data ? `已登记 ${data.assets.length} 个分区` : undefined
            }
          >
            分区 {data?.assets.length ?? "—"}
          </span>
          <span>
            在线{" "}
            {data
              ? data.assets.filter((asset) => asset.connection === "online")
                  .length
              : "—"}
          </span>
        </div>
      </FileStatusBar>
      {removing && (
        <Modal
          title={`移除 ${removing.name} 的登记？`}
          className="asset-dialog"
          onClose={() => setRemoving(null)}
          closeDisabled={working}
          actions={
            <button
              className="danger"
              disabled={disabled}
              onClick={() =>
                void command({
                  unregister: { partition_id: removing.partition_id },
                }).then((success) => {
                  if (success) {
                    setRemoving(null);
                    setSelectedPartition("");
                  }
                })
              }
            >
              移除登记
            </button>
          }
        >
          <p>
            将停止此分区的索引任务并清理 StashFinder 的分区索引。磁盘文件和 NTFS
            日志会保留。
          </p>
          {error && (
            <p className="asset-error" role="alert">
              {error}
            </p>
          )}
        </Modal>
      )}
      {rename && (
        <Modal
          title="修改分区名称"
          className="asset-dialog"
          onClose={() => setRename(null)}
          closeDisabled={disabled}
          initialFocus={renameInput}
          actions={
            <button
              type="submit"
              form="partition-rename-form"
              disabled={
                disabled || !name.trim() || Array.from(name.trim()).length > 128
              }
            >
              保存名称
            </button>
          }
        >
          <form
            id="partition-rename-form"
            onSubmit={(e) => {
              e.preventDefault();
              void command({
                rename: { partition_id: rename.partition_id, name },
              }).then((success) => {
                if (success) setRename(null);
              });
            }}
          >
            <label>
              分区名称
              <input
                ref={renameInput}
                value={name}
                disabled={disabled}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <p>仅修改软件内名称，不修改硬盘卷标。</p>
            {error && <p className="asset-error">{error}</p>}
          </form>
        </Modal>
      )}
      {weakConfirm && (
        <Modal
          title="身份信息不足"
          className="asset-dialog"
          onClose={() => {
            setWeakConfirm(false);
            setRegisterOpen(true);
          }}
          actions={
            <button disabled={disabled} onClick={() => void register(true)}>
              按新分区登记
            </button>
          }
        >
          <p>
            这些分区可以按新分区登记，但再次接入时可能无法自动识别。不会自动关联已有记录。
          </p>
        </Modal>
      )}
    </div>
  );
}
