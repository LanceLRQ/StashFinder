import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import Modal from "./Modal";
import type {
  EntryRef,
  FileAction,
  OperationStatus,
  PreparedSummary,
  SelectionInput,
  ReportPage,
} from "./file-operations";
import "./file-operations.css";
const Context = createContext({
  run: (
    _selection: SelectionInput,
    _action: FileAction,
    _changed?: () => void,
  ) => {},
  paste: (_target: EntryRef, _changed?: () => void) => {},
});
const StatusContext = createContext<ReactNode>(null);
const feedbackDuration = 8000;

export const useFileOperations = () => useContext(Context);

export function FileStatusBar({
  children,
  className = "browse-status",
}: {
  children?: ReactNode;
  className?: string;
}) {
  const status = useContext(StatusContext);
  if (!children && !status) return null;
  return (
    <footer className={className}>
      {children}
      {status}
    </footer>
  );
}

export default function FileOperations({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<OperationStatus | null>(null);
  const [message, setMessage] = useState("");
  const [quietMenu, setQuietMenu] = useState(false);
  const [error, setError] = useState("");
  const [report, setReport] = useState<ReportPage | null>(null);
  const [showReport, setShowReport] = useState(false);
  const [confirm, setConfirm] = useState<{
    summary: PreparedSummary;
    action: FileAction;
    changed?: () => void;
  } | null>(null);
  const sequence = useRef(0);
  const running = useRef(false);
  const active =
    !!status &&
    ["preparing", "validating", "executing", "cancelling"].includes(
      status.phase,
    );
  useEffect(() => {
    if (active || !(error || message)) return;
    const operation = sequence.current;
    const timer = window.setTimeout(() => {
      if (sequence.current === operation) {
        setMessage("");
        setError("");
      }
    }, feedbackDuration);
    return () => window.clearTimeout(timer);
  }, [active, error, message]);
  useEffect(() => {
    if (!active) return;
    let live = true;
    const timer = window.setInterval(() => {
      void invoke<OperationStatus>("file_status")
        .then((s) => {
          if (live) setStatus(s);
        })
        .catch((e) => {
          if (live) setError(String(e));
        });
    }, 300);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [active]);
  async function readReport(
    token: number,
    id: number | null = null,
    offset = 0,
  ) {
    const page = await invoke<ReportPage | null>("file_report", { id, offset });
    if (page && token === sequence.current)
      setReport((previous) =>
        offset && previous && previous.id === page.id
          ? { ...page, rows: [...previous.rows, ...page.rows] }
          : page,
      );
    return page;
  }
  async function execute(
    summary: PreparedSummary,
    action: FileAction,
    changed?: () => void,
  ) {
    const token = sequence.current;
    setConfirm(null);
    setStatus({
      id: summary.id,
      phase: "executing",
      total: summary.count,
      processed: 0,
      error: null,
    });
    try {
      const result = await invoke<string>("file_run", {
        id: summary.id,
        action,
      });
      if (token === sequence.current) setMessage(result);
    } finally {
      if (token === sequence.current) {
        setStatus(null);
        const page = await readReport(token, summary.id).catch((e) => {
          if (token === sequence.current)
            setError((previous) => previous || String(e));
        });
        if (
          token === sequence.current &&
          page?.completed &&
          (action === "recycle" ||
            action === "delete" ||
            (typeof action === "object" && "rename" in action))
        )
          changed?.();
      }
    }
  }
  function fail(e: unknown, token: number) {
    if (token === sequence.current) {
      setError(String(e).replace(/^Error: /, ""));
      setStatus(null);
      setConfirm(null);
      running.current = false;
    }
  }
  async function run(
    selection: SelectionInput,
    action: FileAction,
    changed?: () => void,
  ) {
    if (running.current) return;
    running.current = true;
    const token = ++sequence.current;
    setQuietMenu(typeof action === "object" && "more" in action);
    setMessage("");
    setError("");
    setReport(null);
    setShowReport(false);
    setStatus({
      id: 0,
      phase: "preparing",
      total: 0,
      processed: 0,
      error: null,
    });
    const summary = await invoke<PreparedSummary>("file_prepare", {
      input: selection,
      recordsOnly: action === "copy_name" || action === "copy_path",
    }).catch((e) => {
      fail(e, token);
      return null;
    });
    if (!summary || token !== sequence.current) return;
    if (action === "delete" || action === "recycle") {
      setStatus(null);
      setConfirm({ summary, action, changed });
      return;
    }
    await execute(summary, action, changed).catch((e) => fail(e, token));
    running.current = false;
  }
  async function cancel() {
    if (status?.phase === "executing" || status?.phase === "cancelling") {
      await invoke("file_cancel").catch((e) => setError(String(e)));
      setStatus({ ...status, phase: "cancelling" });
      return;
    }
    ++sequence.current;
    setConfirm(null);
    await invoke("file_cancel").catch((e) => setError(String(e)));
    setStatus(null);
    setMessage("操作已取消");
    running.current = false;
  }
  const api = {
    run: (
      selection: SelectionInput,
      action: FileAction,
      changed?: () => void,
    ) => {
      void run(selection, action, changed);
    },
    paste: (target: EntryRef, changed?: () => void) => {
      if (running.current) return;
      running.current = true;
      const token = ++sequence.current;
      setQuietMenu(false);
      setError("");
      setReport(null);
      setShowReport(false);
      setMessage("");
      setStatus({
        id: 0,
        phase: "executing",
        total: 0,
        processed: 0,
        error: null,
      });
      void invoke<string>("file_paste", { target })
        .then((result) => {
          if (token === sequence.current) {
            setMessage(result);
            setStatus(null);
          }
        })
        .catch((e) => fail(e, token))
        .finally(async () => {
          if (token === sequence.current) {
            const page = await readReport(token).catch((e) => {
              if (token === sequence.current)
                setError((previous) => previous || String(e));
            });
            if (token === sequence.current && page?.completed) changed?.();
          }
          if (token === sequence.current) running.current = false;
        });
    },
  };
  const feedback = (error || (!quietMenu && (status || message))) && (
    <div
      className={`file-operation-status ${error ? "has-error" : ""}`}
      aria-label="文件操作状态"
    >
      <span role={error ? "alert" : "status"}>
        {error ||
          message ||
          (status?.phase === "preparing"
            ? "正在准备文件…"
            : status?.phase === "validating"
              ? `正在核验 ${status.processed} / ${status.total}`
              : status?.phase === "cancelling"
                ? "正在取消…"
                : "正在执行文件操作…")}
      </span>
      {report && !active && (
        <button onClick={() => setShowReport(true)}>查看结果</button>
      )}
      {active && <button onClick={() => void cancel()}>取消</button>}
    </div>
  );
  return (
    <Context.Provider value={api}>
      <StatusContext.Provider value={feedback}>
        {children}
      </StatusContext.Provider>
      {showReport && report && (
        <Modal
          title="文件操作结果"
          className="operation-results"
          onClose={() => setShowReport(false)}
          actions={
            report.next !== null && (
              <button
                onClick={() =>
                  void readReport(
                    sequence.current,
                    report.id,
                    report.next!,
                  ).catch((e) => setError(String(e)))
                }
              >
                加载更多结果
              </button>
            )
          }
        >
          <table>
            <thead>
              <tr>
                <th>文件</th>
                <th>结果</th>
                <th>详情</th>
              </tr>
            </thead>
            <tbody>
              {report.rows.map((row) => (
                <tr key={row.index}>
                  <td title={row.source}>{row.source}</td>
                  <td>
                    {
                      {
                        completed: "完成",
                        failed: "失败",
                        skipped: "跳过",
                        not_run: "未执行",
                      }[row.state]
                    }
                  </td>
                  <td>{row.error || row.note || row.destination || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Modal>
      )}
      {status?.close_requested && (
        <Modal
          role="alertdialog"
          title="文件操作仍在进行"
          onClose={() => {
            void invoke("file_keep_open").catch((e) => setError(String(e)));
            setStatus({ ...status, close_requested: false });
          }}
          actions={<button onClick={() => void cancel()}>取消操作</button>}
        >
          <p>关闭此提示后，文件操作会继续。</p>
        </Modal>
      )}
      {confirm && (
        <Modal
          role="alertdialog"
          className="operation-confirm"
          title={`${confirm.action === "delete" ? "永久删除" : "移入回收站"} ${confirm.summary.count.toLocaleString()} 项？`}
          onClose={() => void cancel()}
          actions={
            <button
              className="danger"
              onClick={() => {
                const token = sequence.current;
                void execute(confirm.summary, confirm.action, confirm.changed)
                  .catch((e) => fail(e, token))
                  .finally(() => {
                    running.current = false;
                  });
              }}
            >
              确认{confirm.action === "delete" ? "永久删除" : "删除"}
            </button>
          }
        >
          {confirm.action === "delete" && <p>这些文件将不经过回收站。</p>}
          <ul>
            {confirm.summary.names.map((name, i) => (
              <li key={i}>{name}</li>
            ))}
          </ul>
        </Modal>
      )}
    </Context.Provider>
  );
}
