import { lazy, Suspense, useEffect, useRef, useState } from "react";
import Assets from "./Assets";
import Search from "./Search";
import Settings from "./Settings";
import type { AssetSnapshot } from "./asset-api";
import Tasks from "./Tasks";
import { taskKinds, taskLabels, useTasks } from "./task-api";
import {
  defaultPreferences,
  readPreferences,
  writePreferences,
  type Page,
  type Preferences,
} from "./settings-api";
import logo from "./logo.svg";
import "./desktop.css";

const Development = import.meta.env.DEV ? lazy(() => import("./App")) : null;
const navigation: { id: Page; label: string; path: string }[] = [
  {
    id: "overview",
    label: "概览",
    path: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  },
  { id: "files", label: "文件", path: "M3 5h7l2 3h9v12H3z" },
  {
    id: "tasks",
    label: "任务",
    path: "m3 6 2 2 3-4 M11 6h10 m-18 9 2 2 3-4 M11 15h10",
  },
  { id: "settings", label: "设置", path: "M4 7h16 M4 17h16 M8 4v6 M16 14v6" },
];

export default function Desktop() {
  const [preferences, setPreferences] = useState(defaultPreferences);
  const [loaded, setLoaded] = useState(false);
  const [preferenceError, setPreferenceError] = useState("");
  const [saving, setSaving] = useState(false);
  const saveLock = useRef(false);
  const [page, setPage] = useState<Page>("overview");
  const [snapshot, setSnapshot] = useState<AssetSnapshot | null>(null);
  const [searchVisited, setSearchVisited] = useState(false);
  const [searchRequest, setSearchRequest] = useState({ text: "", revision: 0 });
  const [development, setDevelopment] = useState(false);
  const tasks = useTasks(loaded && page !== "files");
  const [focusTask, setFocusTask] = useState("");
  useEffect(() => {
    let active = true;
    void readPreferences()
      .then(
        (value) => {
          if (!active) return;
          setPreferences(value);
          setPage(value.start_page);
          setSearchVisited(value.start_page === "files");
        },
        (error) => {
          if (active) setPreferenceError(`设置读取失败：${String(error)}`);
        },
      )
      .then(() => {
        if (active) setLoaded(true);
      });
    return () => {
      active = false;
    };
  }, []);
  async function save(next: Preferences) {
    if (saveLock.current) return;
    saveLock.current = true;
    setSaving(true);
    await writePreferences(next).then(
      (value) => {
        setPreferences(value);
        setPreferenceError("");
      },
      (e) => setPreferenceError(`设置未保存：${String(e)}`),
    );
    setSaving(false);
    saveLock.current = false;
  }
  function navigate(next: Page) {
    setDevelopment(false);
    setPage(next);
    if (next === "files") setSearchVisited(true);
  }
  function search(text: string) {
    setSearchRequest((previous) => ({ text, revision: previous.revision + 1 }));
    navigate("files");
  }
  const activity = (
    <>
      <div className="activity-heading">
        <h2>任务动态</h2>
        <button
          onClick={() => {
            setFocusTask("");
            navigate("tasks");
          }}
        >
          全部
        </button>
      </div>
      {tasks.error && (
        <p className="desktop-error" role="alert">
          {tasks.error}
        </p>
      )}
      {tasks.data?.tasks.slice(0, 5).map((task) => (
        <div className="activity-row" key={task.id}>
          <button
            className="task-name"
            onClick={() => {
              setFocusTask(task.id);
              navigate("tasks");
            }}
          >
            {snapshot?.assets.find(
              (a) => a.registration.partition_id === task.partition_id,
            )?.registration.name || task.partition_id}
          </button>
          <span>
            {taskKinds[task.kind] || task.kind} ·{" "}
            {taskLabels[task.state] || task.state}
          </span>
          <small>{new Date(task.updated * 1000).toLocaleString()}</small>
          {task.error && <p className="desktop-error">{task.error}</p>}
        </div>
      ))}
      {tasks.data && !tasks.data.tasks.length && (
        <p className="stage-note">当前没有任务</p>
      )}
      {!tasks.data && !tasks.error && (
        <p className="stage-note">正在读取任务…</p>
      )}
    </>
  );
  if (!loaded)
    return (
      <div className="desktop-loading" role="status">
        正在读取设置…
      </div>
    );
  return (
    <div className={`desktop ${preferences.collapsed ? "rail-collapsed" : ""}`}>
      <aside className="function-rail">
        <div className="rail-brand">
          <img src={logo} alt="" />
          <span>StashFinder</span>
        </div>
        <button
          className="rail-toggle"
          aria-expanded={!preferences.collapsed}
          disabled={saving}
          title={preferences.collapsed ? "展开功能栏" : "收起功能栏"}
          aria-label={preferences.collapsed ? "展开功能栏" : "收起功能栏"}
          onClick={() =>
            void save({ ...preferences, collapsed: !preferences.collapsed })
          }
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 5h16 M4 12h16 M4 19h16" />
          </svg>
        </button>
        <nav aria-label="主功能">
          {navigation.map((item) => (
            <button
              key={item.id}
              title={item.label}
              aria-label={item.label}
              aria-current={
                page === item.id && !development ? "page" : undefined
              }
              onClick={() => navigate(item.id)}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d={item.path} />
              </svg>
              <span>{item.label}</span>
            </button>
          ))}
        </nav>
        {Development && (
          <button
            className="development-entry"
            title="开发验证"
            onClick={() => setDevelopment(true)}
          >
            <span>开发验证</span>
          </button>
        )}
      </aside>
      <main className="desktop-main">
        {preferenceError && (
          <p role="alert" className="desktop-error preference-error">
            {preferenceError}
          </p>
        )}
        <div
          className="desktop-page"
          hidden={page !== "overview" || development}
        >
          <Assets
            onSearch={search}
            onSnapshot={setSnapshot}
            activity={activity}
          />
        </div>
        {searchVisited && (
          <div
            className="desktop-page"
            hidden={page !== "files" || development}
          >
            <Search
              partitions={(snapshot?.assets || []).map((a) => ({
                id: a.registration.partition_id,
                name: a.registration.name,
              }))}
              includePath={preferences.include_path}
              request={searchRequest}
            />
          </div>
        )}
        <div
          className="desktop-page tasks-page"
          hidden={page !== "tasks" || development}
        >
          {page === "tasks" && (
            <Tasks
              key={focusTask}
              focusTask={focusTask}
              data={tasks.data}
              error={tasks.error}
              onRefresh={tasks.refresh}
              partitions={(snapshot?.assets || []).map((a) => ({
                id: a.registration.partition_id,
                name: a.registration.name,
                connection: a.connection,
              }))}
            />
          )}
        </div>
        {page === "settings" && !development && (
          <Settings
            preferences={preferences}
            saving={saving}
            onSave={save}
            tasks={tasks.data}
            taskError={tasks.error}
            onTasksRefresh={tasks.refresh}
          />
        )}
        {development && Development && (
          <Suspense fallback={<p>正在加载验证工具…</p>}>
            <Development />
          </Suspense>
        )}
      </main>
    </div>
  );
}
