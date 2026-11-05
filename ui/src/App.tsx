import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import SampleList, { type Sample, type FileAction } from "./SampleList";
import "./style.css";

type Status = {
  mode: string;
  process_id: number;
  registered_partitions: number;
};

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [connectionError, setConnectionError] = useState("");
  const [checking, setChecking] = useState(false);
  const [samples, setSamples] = useState<Sample[]>([]);
  const [selected, setSelected] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);
  const operation = useRef(false);
  const [message, setMessage] = useState("尚未准备样本。");
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState<{
    action: "rename" | "recycle" | "delete";
    ids: number[];
  } | null>(null);
  const [newName, setNewName] = useState("");

  async function checkConnection() {
    setChecking(true);
    setStatus(null);
    setConnectionError("");
    await invoke<Status>("service_status").then(setStatus, (error) =>
      setConnectionError(String(error)),
    );
    setChecking(false);
  }

  async function sampleAction(
    action: "create_samples" | FileAction,
    ids = selected,
    name?: string,
  ) {
    if (operation.current) return;
    operation.current = true;
    setBusy(true);
    setFailed(false);
    setMessage(
      action === "drag_samples"
        ? "拖到目标文件夹后松开；按 Esc 可取消。"
        : "正在处理…",
    );
    await (
      action === "create_samples" || action === "refresh_samples"
        ? invoke<Sample[]>(action).then((items) => {
            setSamples(items);
            setSelected([]);
            setMessage("样本状态已更新；不可用条目保留供核对。");
          })
        : ["rename", "recycle", "delete"].includes(action)
          ? invoke<string>("edit_samples", { ids, action, name }).then(
              setMessage,
            )
          : invoke<string>(action, { ids }).then(setMessage)
    ).catch((error) => {
      setFailed(true);
      setMessage(String(error));
    });
    if (["rename", "recycle", "delete"].includes(action)) {
      await invoke<Sample[]>("refresh_samples").then(setSamples, (error) => {
        setFailed(true);
        setMessage((previous) => `${previous} 刷新失败：${String(error)}`);
      });
    }
    setBusy(false);
    operation.current = false;
    document.querySelector<HTMLElement>(".sample-list")?.focus();
  }

  function requestAction(action: FileAction, ids: number[]) {
    if (action === "rename" || action === "recycle" || action === "delete") {
      setPending({ action, ids: [...ids] });
      setNewName(
        samples
          .find((sample) => sample.id === ids[0])
          ?.path.split(/[/\\]/)
          .pop() ?? "",
      );
    } else void sampleAction(action, ids);
  }

  function closeDialog() {
    setPending(null);
    document.querySelector<HTMLElement>(".sample-list")?.focus();
  }

  return (
    <div className="development-app">
      <header>
        <span className="wordmark">
          StashFinder<span className="dot">.</span>
        </span>
        <span className="badge">M0 / 开发验证</span>
      </header>
      <h1>验证桌面基础能力</h1>
      <p className="intro">
        检查后台连接与 Windows 文件交互。此版本尚未提供硬盘登记或文件搜索。
      </p>
      <section aria-labelledby="connection-title">
        <div className="section-heading">
          <h2 id="connection-title">
            <span className="number">01</span> 后台连接
          </h2>
          <button onClick={() => void checkConnection()} disabled={checking}>
            {checking ? "连接中…" : "检查连接"}
          </button>
        </div>
        <div aria-live="polite" className="connection">
          {status ? (
            <>
              <strong className="success">已连接</strong>
              <span>
                {status.mode === "windows_service"
                  ? "Windows 后台服务"
                  : "前台验证进程"}{" "}
                · PID {status.process_id} · 登记分区{" "}
                {status.registered_partitions}
              </span>
            </>
          ) : (
            <>
              <strong>{checking ? "正在检查" : "未连接"}</strong>
              <span>
                {connectionError || "点击检查连接，读取实际运行状态。"}
              </span>
            </>
          )}
        </div>
      </section>
      <section aria-labelledby="samples-title">
        <div className="section-heading">
          <h2 id="samples-title">
            <span className="number">02</span> 文件操作验证
          </h2>
          <button
            disabled={busy || samples.length > 0}
            onClick={() => void sampleAction("create_samples")}
          >
            准备验证样本
          </button>
        </div>
        <p>右键查看操作；按住选中行可拖出。</p>
        {samples.length === 0 ? (
          <div className="empty">准备样本后，文件和目录将在这里显示。</div>
        ) : (
          <SampleList
            samples={samples}
            selected={selected}
            disabled={busy || pending !== null}
            onSelect={setSelected}
            onTransfer={requestAction}
          />
        )}
        <div className="actions">
          <span>已选择 {selected.length} 项</span>
          <span>{samples.length} 项 · Ctrl+A 全选 · Esc 取消选择</span>
        </div>
        <p role="status" className={failed ? "message error" : "message"}>
          {message}
        </p>
      </section>
      {pending && (
        <div className="dialog-backdrop">
          <form
            role="dialog"
            aria-modal="true"
            aria-labelledby="operation-title"
            className="operation-dialog"
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                closeDialog();
              }
              if (event.key === "Tab") {
                const controls =
                  event.currentTarget.querySelectorAll<HTMLElement>(
                    "input, button",
                  );
                const first = controls[0],
                  last = controls[controls.length - 1];
                if (event.shiftKey && document.activeElement === first) {
                  event.preventDefault();
                  last.focus();
                } else if (!event.shiftKey && document.activeElement === last) {
                  event.preventDefault();
                  first.focus();
                }
              }
            }}
            onSubmit={(event) => {
              event.preventDefault();
              const { action, ids } = pending;
              setPending(null);
              void sampleAction(
                action,
                ids,
                action === "rename" ? newName : undefined,
              );
            }}
          >
            <h2 id="operation-title">
              {pending.action === "rename"
                ? "重命名"
                : pending.action === "delete"
                  ? "永久删除"
                  : "移入回收站"}
            </h2>
            <p>
              {pending.action === "delete"
                ? "将永久删除所选样本及目录内容，不经过回收站。"
                : pending.action === "recycle"
                  ? "将所选样本及目录内容移入回收站。如 Windows 提示无法回收，请取消。"
                  : "输入完整新名称（包括扩展名）。已有同名项目不会被覆盖。"}
            </p>
            <ul>
              {samples
                .filter((sample) => pending.ids.includes(sample.id))
                .map((sample) => (
                  <li key={sample.id}>{sample.path}</li>
                ))}
            </ul>
            {pending.action === "rename" && (
              <label>
                新名称
                <input
                  autoFocus
                  value={newName}
                  onFocus={(event) => event.target.select()}
                  onChange={(event) => setNewName(event.target.value)}
                />
              </label>
            )}
            <div className="dialog-actions">
              <button
                type="button"
                autoFocus={pending.action !== "rename"}
                onClick={closeDialog}
              >
                取消
              </button>
              <button type="submit">
                {pending.action === "rename"
                  ? "确定"
                  : pending.action === "delete"
                    ? "永久删除"
                    : "移入回收站"}
              </button>
            </div>
          </form>
        </div>
      )}
      <footer>仅用于可丢弃样本。拖出仅复制。</footer>
    </div>
  );
}
