import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import CategoryEditor from "./CategoryEditor";
import CacheSettings from "./CacheSettings";
import Modal from "./Modal";
import type { Preferences, ServiceStatus } from "./settings-api";
import { taskCommand, type TaskSnapshot } from "./task-api";

export default function Settings({
  preferences,
  saving,
  onSave,
  tasks,
  taskError,
  onTasksRefresh,
}: {
  preferences: Preferences;
  saving: boolean;
  onSave: (next: Preferences) => Promise<void>;
  tasks: TaskSnapshot | null;
  taskError: string;
  onTasksRefresh: () => void;
}) {
  const [category, setCategory] = useState("search");
  const [status, setStatus] = useState<ServiceStatus | null>(null);
  const [error, setError] = useState("");
  const [version, setVersion] = useState("");
  const [versionError, setVersionError] = useState("");
  const [checking, setChecking] = useState(false);
  const sequence = useRef(0);
  const [confirmDisable, setConfirmDisable] = useState(false);
  const [policySaving, setPolicySaving] = useState(false);
  const [policyError, setPolicyError] = useState("");
  async function setAutomatic(enabled: boolean) {
    if (policySaving) return;
    setPolicySaving(true);
    setPolicyError("");
    await taskCommand({ auto_update: { enabled } }).then(
      () => {
        setConfirmDisable(false);
        onTasksRefresh();
      },
      (e) => setPolicyError(String(e)),
    );
    setPolicySaving(false);
  }
  async function connect() {
    const id = ++sequence.current;
    setChecking(true);
    setError("");
    setStatus(null);
    await invoke<ServiceStatus>("service_status").then(
      (next) => {
        if (id === sequence.current) setStatus(next);
      },
      (e) => {
        if (id === sequence.current) setError(String(e));
      },
    );
    if (id === sequence.current) setChecking(false);
  }
  useEffect(() => {
    let active = true;
    void invoke<string>("app_version").then(
      (v) => {
        if (active) setVersion(v);
      },
      (e) => {
        if (active) setVersionError(String(e));
      },
    );
    void connect();
    return () => {
      active = false;
      sequence.current++;
    };
  }, []);
  return (
    <div className="settings-page">
      <nav className="settings-nav" aria-label="设置分类">
        {[
          ["search", "搜索"],
          ["sync", "索引与同步"],
          ["cache", "缓存"],
          ["about", "关于与诊断"],
        ].map(([id, label]) => (
          <button
            key={id}
            aria-current={category === id ? "page" : undefined}
            onClick={() => setCategory(id)}
          >
            {label}
          </button>
        ))}
      </nav>
      <div className="settings-content">
        {category === "cache" && <CacheSettings />}
        {category === "sync" && (
          <>
            <label className="setting-row">
              <span>
                自动更新已登记分区
                <small>已在执行的任务会继续，关闭窗口不停止后台服务。</small>
              </span>
              <input
                type="checkbox"
                role="switch"
                aria-label="自动更新已登记分区"
                checked={tasks?.automatic ?? false}
                disabled={!tasks || !!taskError || policySaving}
                onChange={(e) =>
                  e.target.checked
                    ? void setAutomatic(true)
                    : setConfirmDisable(true)
                }
              />
            </label>
            {(taskError || (!confirmDisable && policyError)) && (
              <p role="alert" className="desktop-error">
                {taskError || policyError}
              </p>
            )}
            <p className="stage-note">
              日志断档时自动校验；创建 USN
              日志仍需按分区授权。扫描模式在线监控变化，重连后完整校验。
            </p>
          </>
        )}
        {category === "search" && (
          <>
            <label className="setting-row">
              <span>
                默认包含路径<small>用于下次打开的搜索页面。</small>
              </span>
              <input
                type="checkbox"
                checked={preferences.include_path}
                disabled={saving}
                onChange={(e) =>
                  void onSave({
                    ...preferences,
                    include_path: e.target.checked,
                  })
                }
                aria-label="默认包含路径"
              />
            </label>
            <CategoryEditor
              categories={preferences.categories}
              saving={saving}
              onSave={(categories) => onSave({ ...preferences, categories })}
            />
          </>
        )}
        {category === "about" && (
          <>
            <dl className="diagnostics">
              <div>
                <dt>StashFinder</dt>
                <dd>
                  {version || (versionError ? "读取失败" : "版本读取中…")}
                </dd>
              </div>
              <div>
                <dt>后台服务</dt>
                <dd>{checking ? "正在连接…" : status ? "已连接" : "未连接"}</dd>
              </div>
              {status && (
                <>
                  <div>
                    <dt>运行方式</dt>
                    <dd>{status.mode}</dd>
                  </div>
                  <div>
                    <dt>进程 ID</dt>
                    <dd>{status.process_id}</dd>
                  </div>
                  <div>
                    <dt>协议版本</dt>
                    <dd>{status.version}</dd>
                  </div>
                  <div>
                    <dt>登记分区</dt>
                    <dd>{status.registered_partitions}</dd>
                  </div>
                </>
              )}
            </dl>
            {(error || versionError) && (
              <p role="alert" className="desktop-error">
                {error || versionError}
              </p>
            )}
            <button disabled={checking} onClick={() => void connect()}>
              重新连接
            </button>
            {status?.mode === "portable" && (
              <button
                onClick={() =>
                  void invoke("app_exit").catch((failure) =>
                    setError(String(failure)),
                  )
                }
              >
                退出便携版
              </button>
            )}
          </>
        )}
      </div>
      {confirmDisable && (
        <Modal
          className="asset-dialog"
          title="关闭自动更新？"
          onClose={() => setConfirmDisable(false)}
          closeDisabled={policySaving}
          actions={
            <button
              disabled={policySaving}
              onClick={() => void setAutomatic(false)}
            >
              关闭自动更新
            </button>
          }
        >
          <p>
            之后的文件变化不会自动进入索引；已有任务继续，离线记录仍可搜索。
          </p>
          {policyError && (
            <p role="alert" className="desktop-error">
              {policyError}
            </p>
          )}
        </Modal>
      )}
    </div>
  );
}
