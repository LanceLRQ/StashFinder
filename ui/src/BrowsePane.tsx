import { useEffect, useRef, useState } from "react";
import type { Asset } from "./asset-api";
import {
  connectionLabel,
  readDirectory,
  type DirectoryPage,
} from "./browse-api";
import DirectoryTree from "./DirectoryTree";
import FileList from "./FileList";
import FileDetails from "./FileDetails";

export default function BrowsePane({
  id,
  asset,
  assets,
  initialParent,
  focusEntry,
  onChoose,
  onSearch,
  onTitle,
}: {
  id: number;
  asset?: Asset;
  assets: Asset[];
  initialParent: number[] | null;
  focusEntry?: number[];
  onChoose: (partition: string) => void;
  onSearch: (text: string) => void;
  onTitle: (id: number, title: string) => void;
}) {
  const [parent, setParent] = useState(initialParent);
  const [page, setPage] = useState<DirectoryPage | null>(null);
  const [selection, setSelection] = useState<string[]>(
    focusEntry ? [focusEntry.join(",")] : [],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [tree, setTree] = useState(true);
  const [preview, setPreview] = useState(true);
  const [breadcrumb, setBreadcrumb] = useState(true);
  const [searchText, setSearchText] = useState("");
  const composing = useRef(false);
  const request = useRef(0);
  const currentPage = useRef(page);
  currentPage.current = page;
  const partition = asset?.registration.partition_id;
  const filesystem = asset?.registration.filesystem_id;
  const parentKey = JSON.stringify(parent);
  useEffect(() => {
    if (!partition || !filesystem) return;
    const token = ++request.current;
    setBusy(true);
    setError("");
    async function load() {
      let result: DirectoryPage;
      try {
        result = await readDirectory({
          partition_id: partition!,
          filesystem_id: filesystem!,
          parent: JSON.parse(parentKey),
        });
      } catch (e) {
        if (token !== request.current) return;
        if (!String(e).includes("目录已不存在")) throw e;
        // Only a confirmed missing directory permits ancestor fallback.
        const ancestors = [...(currentPage.current?.ancestors || [])].reverse();
        let recovered: DirectoryPage | null = null;
        for (const ancestor of [...ancestors, { entry: null }]) {
          try {
            recovered = await readDirectory({
              partition_id: partition!,
              filesystem_id: filesystem!,
              parent: ancestor.entry,
            });
            break;
          } catch (fallbackError) {
            if (!String(fallbackError).includes("目录已不存在"))
              throw fallbackError;
          }
        }
        if (!recovered) throw e;
        result = recovered;
        if (token !== request.current) return;
        setSelection([]);
        setNotice("目录已不存在，已返回可用的上级目录");
      }
      if (token !== request.current) return;
      setPage(result);
      setSelection((previous) =>
        previous.filter((key) =>
          result.entries.some((row) => row.entry.join(",") === key),
        ),
      );
      setBusy(false);
    }
    void load().catch((e) => {
      if (token === request.current) {
        setError(String(e).replace(/^Error: /, ""));
        setBusy(false);
      }
    });
    return () => {
      request.current++;
    };
  }, [partition, filesystem, parentKey, refresh]);
  useEffect(() => {
    onTitle(
      id,
      page && page.ancestors.length > 1
        ? page.ancestors[page.ancestors.length - 1].name
        : asset?.registration.name || "浏览",
    );
  }, [id, page, asset?.registration.name, onTitle]);
  function navigate(next: number[] | null) {
    setSelection([]);
    setNotice("");
    setParent(next);
    setRefresh((n) => n + 1);
  }
  function reload() {
    setParent(page?.location || parent);
    setRefresh((n) => n + 1);
  }
  async function more() {
    if (!page || page.next === null || busy) return;
    const token = request.current;
    setBusy(true);
    setError("");
    await readDirectory({
      partition_id: partition!,
      filesystem_id: filesystem!,
      parent: page.location,
      generation: page.generation,
      revision: page.revision,
      offset: page.next,
    }).then(
      (value) => {
        if (token === request.current) {
          setPage({ ...value, entries: [...page.entries, ...value.entries] });
          setBusy(false);
        }
      },
      (e) => {
        if (token === request.current) {
          setError(String(e).replace(/^Error: /, ""));
          setBusy(false);
        }
      },
    );
  }
  return (
    <div
      className="browse-pane"
      onKeyDown={(e) => {
        if (e.key === "F5") {
          e.preventDefault();
          reload();
        }
        if (
          e.altKey &&
          e.key === "ArrowUp" &&
          page &&
          page.ancestors.length > 1
        ) {
          e.preventDefault();
          navigate(page.ancestors[page.ancestors.length - 2].entry);
        }
      }}
    >
      <div className="browse-toolbar">
        <input
          type="search"
          aria-label="搜索文件或路径"
          placeholder="搜索所有文件名或路径…"
          value={searchText}
          onCompositionStart={() => {
            composing.current = true;
          }}
          onCompositionEnd={(e) => {
            composing.current = false;
            if (e.currentTarget.value) {
              onSearch(e.currentTarget.value);
              setSearchText("");
            }
          }}
          onChange={(e) => {
            setSearchText(e.target.value);
            if (!composing.current && e.target.value) {
              onSearch(e.target.value);
              setSearchText("");
            }
          }}
        />
        <select
          aria-label="浏览分区"
          value={partition || ""}
          onChange={(e) => onChoose(e.target.value)}
        >
          <option value="" disabled>
            选择分区
          </option>
          {assets.map((a) => (
            <option
              key={a.registration.partition_id}
              value={a.registration.partition_id}
            >
              {a.registration.name}
            </option>
          ))}
        </select>
        <button
          aria-label="刷新目录"
          title="刷新目录 (F5)"
          disabled={!asset}
          onClick={reload}
        >
          刷新
        </button>
        <details className="view-menu">
          <summary>视图</summary>
          <div>
            <label>
              <input
                type="checkbox"
                checked={tree}
                onChange={(e) => setTree(e.target.checked)}
              />
              目录树
            </label>
            <label>
              <input
                type="checkbox"
                checked={preview}
                onChange={(e) => setPreview(e.target.checked)}
              />
              预览窗格
            </label>
            <label>
              <input
                type="checkbox"
                checked={breadcrumb}
                onChange={(e) => setBreadcrumb(e.target.checked)}
              />
              面包屑
            </label>
          </div>
        </details>
        <span className="browse-count" role="status">
          {busy
            ? "正在读取…"
            : selection.length
              ? `已选择 ${selection.length} 项`
              : page
                ? `${page.entries.length}${page.next !== null ? "+" : ""} 项`
                : ""}
        </span>
      </div>
      {!asset ? (
        <p className="search-empty">选择已登记分区以浏览文件</p>
      ) : (
        <>
          {error && (
            <p className="browse-message desktop-error" role="alert">
              {error}
            </p>
          )}
          {notice && (
            <p className="browse-message" role="status">
              {notice}
            </p>
          )}
          <div className="browse-body">
            {tree && (
              <nav className="directory-tree" aria-label="目录树">
                <DirectoryTree
                  partition={partition!}
                  filesystem={filesystem!}
                  name={asset.registration.name}
                  selected={page?.location || null}
                  ancestors={page?.ancestors}
                  version={page ? `${page.generation}:${page.revision}` : ""}
                  onNavigate={navigate}
                />
              </nav>
            )}
            <div className="directory-main">
              {breadcrumb && (
                <nav className="breadcrumbs" aria-label="目录位置">
                  <button
                    aria-label="上一级"
                    disabled={!page || page.ancestors.length < 2}
                    onClick={() =>
                      navigate(
                        page!.ancestors[page!.ancestors.length - 2].entry,
                      )
                    }
                  >
                    ↑
                  </button>
                  {(page?.ancestors || [{ entry: [], name: "" }]).map(
                    (a, i) => (
                      <span key={a.entry.join(",")}>
                        {i > 0 && <span aria-hidden="true"> / </span>}
                        <button
                          title={i ? a.name : asset.registration.name}
                          onClick={() => navigate(i ? a.entry : null)}
                        >
                          {i ? a.name : asset.registration.name}
                        </button>
                      </span>
                    ),
                  )}
                </nav>
              )}
              {page && (
                <FileList
                  key={page.location.join(",")}
                  rows={page.entries}
                  selection={selection}
                  onSelect={setSelection}
                  onOpen={(row) => {
                    if (row.kind === "folder") navigate(row.entry);
                  }}
                  next={page.next}
                  busy={busy}
                  onMore={() => void more()}
                />
              )}
              {!page && !busy && !error && (
                <p className="search-empty">暂无可浏览的索引</p>
              )}
            </div>
            {preview && (
              <FileDetails
                rows={
                  page?.entries.filter((row) =>
                    selection.includes(row.entry.join(",")),
                  ) || []
                }
                partition={asset.registration.name}
                connection={asset.connection}
                indexedAt={page?.indexed_at ?? null}
              />
            )}
          </div>
          <footer className="browse-status">
            <span>
              {connectionLabel[asset.connection] || "状态未确认"}
              {asset.connection === "online" && asset.mounts.length > 0
                ? ` · ${asset.mounts.join("、")}`
                : ""}
            </span>
            <span>
              {page?.partial
                ? "未完成记录"
                : page?.needs_validation
                  ? "索引待校验"
                  : ""}
              {page?.indexed_at
                ? ` · 索引 ${new Date(page.indexed_at * 1000).toLocaleString()}`
                : ""}
            </span>
          </footer>
        </>
      )}
    </div>
  );
}
