import { useEffect, useState } from "react";
import { indexRequest, type SearchHit } from "./index-api";
import "./assets.css";
import "./search.css";

function fileTime(value: string | null) {
  if (value == null) return "未知";
  const milliseconds = Number(BigInt(value) / 10000n - 11644473600000n);
  return new Date(milliseconds).toLocaleString();
}
const connection: Record<string, string> = {
  online: "在线",
  offline: "离线",
  uncertain: "身份待确认",
  changed: "历史记录",
  historical: "历史记录",
  inaccessible: "无法访问",
  unconfirmed: "状态未确认",
};
export default function Search({
  partitions,
  onBack,
  includePath = true,
  request,
}: {
  partitions: { id: string; name: string }[];
  onBack?: () => void;
  includePath?: boolean;
  request?: { text: string; revision: number };
}) {
  const [text, setText] = useState(request?.text || "");
  const [namesOnly, setNamesOnly] = useState(!includePath);
  const [partition, setPartition] = useState("");
  const [rows, setRows] = useState<SearchHit[]>([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [unavailable, setUnavailable] = useState<string[]>([]);
  const [more, setMore] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (request) {
      setText(request.text);
      setRevision((value) => value + 1);
    }
  }, [request]);
  useEffect(() => {
    let active = true;
    let id: number | undefined;
    setBusy(true);
    setError("");
    setRows([]);
    setUnavailable([]);
    setMore(false);
    async function run() {
      const start = await indexRequest({
        search_start: {
          text,
          names_only: namesOnly,
          partitions: partition ? [partition] : [],
        },
      });
      if (start.kind !== "search_started") throw new Error("搜索响应无效");
      id = start.data;
      if (!active) {
        await indexRequest({ search_cancel: { id } });
        return;
      }
      const results: SearchHit[] = [];
      let offset = 0;
      while (active) {
        const reply = await indexRequest({ search_page: { id, offset } });
        if (!active) return;
        if (reply.kind !== "search_page") throw new Error("搜索分页响应无效");
        const page = reply.data;
        if (page.error) throw new Error(page.error);
        if (!page.done) {
          await new Promise((r) => window.setTimeout(r, 100));
          continue;
        }
        if (page.row) results.push(page.row);
        if (page.next != null) {
          offset = page.next;
          continue;
        }
        setRows(results);
        setMore(page.more);
        setUnavailable(page.unavailable);
        setBusy(false);
        return;
      }
    }
    const timer = window.setTimeout(() => {
      void run().catch((e) => {
        if (active) {
          setError(e instanceof Error ? e.message : String(e));
          setBusy(false);
        }
      });
    }, 150);
    return () => {
      active = false;
      window.clearTimeout(timer);
      if (id !== undefined)
        void indexRequest({ search_cancel: { id } }).catch(() => {});
    };
  }, [text, namesOnly, partition, revision]);
  return (
    <div className="assets-app search-app">
      {onBack && <button onClick={onBack}>硬盘管理</button>}
      <div className="search-toolbar">
        <input
          autoFocus
          type="search"
          aria-label="搜索文件或路径"
          placeholder="搜索文件名或路径，空格分隔多个关键词"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <select
          aria-label="分区范围"
          value={partition}
          onChange={(e) => setPartition(e.target.value)}
        >
          <option value="">全部已登记分区</option>
          {partitions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <label>
          <input
            type="checkbox"
            checked={namesOnly}
            onChange={(e) => setNamesOnly(e.target.checked)}
          />
          仅文件名
        </label>
        <button onClick={() => setRevision((n) => n + 1)}>重新搜索</button>
        <div className="search-summary" role="status">
          {busy
            ? "正在搜索…"
            : error
              ? "搜索失败"
              : more
                ? `显示前 ${rows.length} 条，请缩小搜索范围`
                : `${rows.length} 条结果`}
        </div>
      </div>
      {error && (
        <p role="alert" className="asset-error">
          {error}
        </p>
      )}
      {unavailable.length > 0 && (
        <details className="search-unavailable" open>
          <summary>{unavailable.length} 个分区未参与搜索</summary>
          {unavailable.map((item, i) => (
            <p key={i}>{item}</p>
          ))}
        </details>
      )}
      <div className="asset-table-wrap search-results">
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>所在目录 / 路径</th>
              <th>分区</th>
              <th>大小</th>
              <th>修改时间</th>
              <th>状态 / 索引时间</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={`${row.partition_id}:${row.file.path}`}>
                <td>
                  <span
                    className={`file-mark ${row.file.kind}`}
                    aria-hidden="true"
                  />
                  {row.file.name}
                </td>
                <td className="result-path" title={row.file.path}>
                  {row.connection === "online" && row.mounts[0]
                    ? row.mounts[0]
                    : ""}
                  {row.file.path}
                </td>
                <td>{row.partition_name}</td>
                <td className="result-size">
                  {row.file.kind === "folder"
                    ? "—"
                    : row.file.size === null
                      ? "未知"
                      : `${BigInt(row.file.size).toLocaleString()} B`}
                </td>
                <td>{fileTime(row.file.modified)}</td>
                <td>
                  <span className={`connection ${row.connection}`}>
                    {connection[row.connection] || "状态未确认"}
                  </span>
                  {row.partial ? (
                    <small>未完成记录</small>
                  ) : row.needs_validation ? (
                    <small>索引待校验</small>
                  ) : null}
                  <small>
                    {row.indexed_at == null
                      ? "索引时间未知"
                      : new Date(row.indexed_at * 1000).toLocaleString()}
                  </small>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!busy && !error && !rows.length && (
          <p className="search-empty">没有匹配的文件</p>
        )}
      </div>
    </div>
  );
}
