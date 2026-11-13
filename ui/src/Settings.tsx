import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import CategoryEditor from "./CategoryEditor";
import CacheSettings from "./CacheSettings";
import Modal from "./Modal";
import {
  defaultPreferences,
  type Preferences,
  type ServiceStatus,
} from "./settings-api";
import { taskCommand, type TaskSnapshot } from "./task-api";
import SearchFilters from "./SearchFilters";
import {
  defaultCategories,
  emptyOptions,
  filterPresetEntries,
  noFilter,
  noPresets,
} from "./search-options";
import { shortcutReference } from "./workspace-shortcuts";

const sections = [
  ["general", "常用"],
  ["categories", "文件分类"],
  ["presets", "筛选预设"],
  ["shortcuts", "快捷键"],
  ["sync", "索引与同步"],
  ["cache", "图片缓存"],
  ["about", "关于与诊断"],
] as const;
export type SettingsPage = (typeof sections)[number][0];
type PreferenceDraft = Pick<
  Preferences,
  | "include_path"
  | "browse_filter"
  | "categories"
  | "filter_presets"
  | "search_online_only"
  | "search_include_history"
  | "default_view"
  | "default_preview"
  | "show_status_bar"
>;

export default function Settings({
  preferences,
  saving,
  onSave,
  tasks,
  taskError,
  onTasksRefresh,
  initialPage = "general",
  onClose = () => {},
  preferenceError = "",
  preferenceUnavailable = false,
}: {
  preferences: Preferences;
  saving: boolean;
  onSave: (next: Partial<PreferenceDraft>) => Promise<unknown>;
  tasks: TaskSnapshot | null;
  taskError: string;
  onTasksRefresh: () => void;
  initialPage?: SettingsPage;
  onClose?: () => void;
  preferenceError?: string;
  preferenceUnavailable?: boolean;
}) {
  const [category, setCategory] = useState<SettingsPage>(initialPage);
  const [changes, setChanges] = useState<Partial<PreferenceDraft>>({});
  const draft = { ...defaultPreferences, ...preferences, ...changes };
  const categories = draft.categories ?? defaultCategories;
  const presets = draft.filter_presets ?? noPresets;
  const availablePresets = filterPresetEntries(categories, presets);
  const selectedPreset =
    availablePresets.find(({ id }) => id === draft.browse_filter)?.id ?? null;
  const formDisabled = saving || preferenceUnavailable;
  const navFocus = useRef<HTMLButtonElement>(null);
  const changed = (Object.keys(changes) as (keyof PreferenceDraft)[]).some(
    (key) =>
      JSON.stringify(changes[key] ?? null) !==
      JSON.stringify(preferences[key] ?? null),
  );
  async function commit() {
    if (changed && (formDisabled || (await onSave(changes)) === false)) return;
    onClose();
  }
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
    if (category !== "about") return;
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
  }, [category]);
  return (
    <Modal
      title="设置"
      className="settings-dialog"
      onClose={onClose}
      initialFocus={navFocus}
      closeDisabled={saving}
      actions={
        <>
          {category === "general" && (
            <button
              type="button"
              className="settings-reset"
              disabled={formDisabled}
              onClick={() =>
                setChanges((previous) => ({
                  ...previous,
                  include_path: defaultPreferences.include_path,
                  browse_filter: null,
                  search_online_only: false,
                  search_include_history: false,
                  default_view: "details",
                  default_preview: false,
                  show_status_bar: true,
                }))
              }
            >
              恢复默认
            </button>
          )}
          <button type="button" disabled={saving} onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="primary"
            disabled={saving || (changed && preferenceUnavailable)}
            onClick={() => void commit()}
          >
            确定
          </button>
        </>
      }
    >
      <div className="settings-page">
        <nav className="settings-nav" aria-label="设置分类">
          {sections.map(([id, label]) => (
            <button
              type="button"
              key={id}
              ref={id === initialPage ? navFocus : undefined}
              aria-current={category === id ? "page" : undefined}
              onClick={() => setCategory(id)}
            >
              {label}
            </button>
          ))}
        </nav>
        <div className="settings-content">
          {preferenceError && (
            <p role="alert" className="desktop-error">
              {preferenceError}
            </p>
          )}
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
              {!tasks && !taskError && <p role="status">自动更新状态未知</p>}
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
          {category === "general" && (
            <>
              <label className="setting-row">
                <span>
                  默认包含路径<small>用于下次打开的搜索页面。</small>
                </span>
                <input
                  type="checkbox"
                  checked={draft.include_path}
                  disabled={formDisabled}
                  onChange={(e) =>
                    setChanges({ ...changes, include_path: e.target.checked })
                  }
                  aria-label="默认包含路径"
                />
              </label>
              <label className="setting-row">
                显示状态栏
                <input
                  type="checkbox"
                  checked={draft.show_status_bar ?? true}
                  disabled={formDisabled}
                  onChange={(e) =>
                    setChanges({
                      ...changes,
                      show_status_bar: e.target.checked,
                    })
                  }
                />
              </label>
              {(
                [
                  ["search_online_only", "新搜索默认仅在线"],
                  ["search_include_history", "新搜索默认包含历史记录"],
                  ["default_preview", "新标签默认显示预览窗格"],
                ] as const
              ).map(([key, label]) => (
                <label className="setting-row" key={key}>
                  {label}
                  <input
                    type="checkbox"
                    checked={draft[key] ?? false}
                    disabled={formDisabled}
                    onChange={(e) =>
                      setChanges({ ...changes, [key]: e.target.checked })
                    }
                  />
                </label>
              ))}
              <label className="setting-row">
                新标签默认布局
                <select
                  disabled={formDisabled}
                  value={draft.default_view ?? "details"}
                  onChange={(e) =>
                    setChanges({
                      ...changes,
                      default_view: e.target
                        .value as Preferences["default_view"],
                    })
                  }
                >
                  <option value="details">详情</option>
                  <option value="large">大图标</option>
                  <option value="medium">中图标</option>
                  <option value="small">小图标</option>
                </select>
              </label>
              <label className="setting-row">
                浏览与新搜索默认筛选
                <select
                  disabled={formDisabled}
                  value={selectedPreset ?? ""}
                  onChange={(e) =>
                    setChanges({
                      ...changes,
                      browse_filter: e.target.value || null,
                    })
                  }
                >
                  {availablePresets.map(({ id, name }) => (
                    <option key={id ?? ""} value={id ?? ""}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
            </>
          )}
          <div hidden={category !== "categories"}>
            <CategoryEditor
              categories={categories}
              saving={formDisabled}
              onSave={async (categories) =>
                setChanges((previous) => ({ ...previous, categories }))
              }
            />
          </div>
          <div hidden={category !== "presets"}>
            <SearchFilters
              mode="manage"
              initialOptions={emptyOptions}
              initialFilter={noFilter}
              initialNamesOnly={false}
              defaultNamesOnly={false}
              categories={categories}
              presets={presets}
              savingPresets={formDisabled}
              onApply={() => {}}
              onClose={() => {}}
              onSavePresets={async (filter_presets) => {
                setChanges((previous) => ({ ...previous, filter_presets }));
                return true;
              }}
            />
          </div>
          {category === "shortcuts" && (
            <div className="shortcut-reference">
              <h3>程序菜单</h3>
              <p>
                Alt+F 文件 · Alt+E 编辑 · Alt+V 查看 · Alt+S 搜索 · Alt+B 书签 ·
                Alt+T 工具 · Alt+H 帮助
              </p>
              {shortcutReference.map(({ label, keys }) => (
                <section key={label}>
                  <h3>{label}</h3>
                  <dl>
                    {keys.map(([label, key]) => (
                      <div key={key}>
                        <dt>{label}</dt>
                        <dd>
                          <kbd>{key}</kbd>
                        </dd>
                      </div>
                    ))}
                  </dl>
                </section>
              ))}
            </div>
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
                  <dd>
                    {checking ? "正在连接…" : status ? "已连接" : "未连接"}
                  </dd>
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
    </Modal>
  );
}
