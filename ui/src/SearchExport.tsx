import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Modal from "./Modal";

export type ExportSnapshot = {
  id: number;
  total: number;
  spans: [number, number][];
  count: number;
};

export default function SearchExport({
  snapshot,
  onClose,
}: {
  snapshot: ExportSnapshot;
  onClose: () => void;
}) {
  const [selected, setSelected] = useState(snapshot.count > 0);
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [progress, setProgress] = useState<[number, number] | null>(null);
  const [result, setResult] = useState<number | null>(null);
  const [error, setError] = useState("");
  const running = useRef(false);
  async function start() {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    setProgress(null);
    let stop: (() => void) | undefined;
    try {
      stop = await listen<[number, number]>(
        "search-export-progress",
        ({ payload }) => setProgress(payload),
      );
      const written = await invoke<number | null>("export_search_csv", {
        id: snapshot.id,
        total: snapshot.total,
        spans: selected ? snapshot.spans : null,
      });
      if (written === null) onClose();
      else setResult(written);
    } catch (error) {
      setError(String(error).replace(/^Error: /, ""));
    } finally {
      stop?.();
      running.current = false;
      setBusy(false);
      setCancelling(false);
    }
  }
  function cancel() {
    if (!busy) {
      onClose();
      return;
    }
    setCancelling(true);
    void invoke("cancel_search_export").catch((failure) => {
      setError(String(failure));
      setCancelling(false);
    });
  }
  return (
    <Modal
      title="导出搜索结果"
      onClose={cancel}
      closeDisabled={busy}
      actions={
        result === null ? (
          <>
            <button
              type="button"
              disabled={cancelling || (busy && progress === null)}
              onClick={cancel}
            >
              {cancelling ? "正在取消…" : "取消"}
            </button>
            <button
              type="button"
              className="primary"
              disabled={busy}
              onClick={() => void start()}
            >
              保存 CSV…
            </button>
          </>
        ) : (
          <button type="button" onClick={onClose}>
            完成
          </button>
        )
      }
    >
      {result !== null ? (
        <p role="status">已导出 {result.toLocaleString()} 条结果。</p>
      ) : (
        <>
          <fieldset disabled={busy}>
            <legend>导出范围</legend>
            <label>
              <input
                type="radio"
                name="export-scope"
                checked={selected}
                disabled={!snapshot.count}
                onChange={() => setSelected(true)}
              />{" "}
              所选结果（{snapshot.count.toLocaleString()}）
            </label>
            <br />
            <label>
              <input
                type="radio"
                name="export-scope"
                checked={!selected}
                onChange={() => setSelected(false)}
              />{" "}
              全部匹配（{snapshot.total.toLocaleString()}）
            </label>
          </fieldset>
          {busy &&
            (progress ? (
              <p role="status">
                已写入 {progress[0].toLocaleString()} /{" "}
                {progress[1].toLocaleString()} 条结果
              </p>
            ) : (
              <p role="status">请选择保存位置…</p>
            ))}
          {error && <p role="alert">{error}</p>}
        </>
      )}
    </Modal>
  );
}
