import { useCallback, useEffect, useRef, useState } from "react";
import { indexRequest, type SearchHit } from "./index-api";
import { connectionLabel, fileTime, fileSize } from "./browse-api";
import {
  defaultCategories,
  emptyOptions,
  type Category,
  type SearchContext,
  type SearchOptions,
} from "./search-options";
import FileDetails from "./FileDetails";
import SearchFilters from "./SearchFilters";
import VirtualResults, { virtualScale } from "./VirtualResults";
import FileContextMenu, { type MenuAction } from "./FileContextMenu";
import RenameInput from "./RenameInput";
import type { Asset } from "./asset-api";
import { useFileOperations } from "./FileOperations";
import {
  fileKey,
  type FileAction,
  type SelectionInput,
} from "./file-operations";
import "./assets.css";
import "./search.css";
import "./search-advanced.css";
type Selection = { range: [number, number] | null; toggles: Set<number> };
const emptySelection = (): Selection => ({ range: null, toggles: new Set() });
export default function Search({
  partitions,
  assets,
  onBack,
  includePath = true,
  request,
  active = true,
  onLocate,
  categories = defaultCategories,
  context,
  indexVersion = "",
}: {
  partitions: { id: string; name: string }[];
  assets?: Asset[];
  onBack?: () => void;
  includePath?: boolean;
  request?: { text: string; revision: number };
  active?: boolean;
  onLocate?: (hit: SearchHit, folder?: boolean) => void;
  categories?: Category[];
  context?: SearchContext;
  indexVersion?: string;
}) {
  function currentHit(hit: SearchHit): SearchHit {
    if (assets === undefined) return hit;
    const asset = assets.find(
      (a) => a.registration.partition_id === hit.partition_id,
    );
    const sameFilesystem =
      asset?.registration.filesystem_id === hit.filesystem_id;
    const connection = !asset
      ? "unconfirmed"
      : !sameFilesystem
        ? "historical"
        : asset.connection === "online" && asset.registration.state !== "ready"
          ? "unconfirmed"
          : asset.connection;
    return {
      ...hit,
      partition_name: asset?.registration.name ?? hit.partition_name,
      connection,
      mounts: connection === "online" ? asset!.mounts : [],
    };
  }
  const input = useRef<HTMLInputElement>(null);
  const operations = useFileOperations();
  const [editing, setEditing] = useState<number | null>(null);
  const lastIndex = useRef(indexVersion);
  useEffect(() => {
    if (lastIndex.current !== indexVersion && editing === null) {
      lastIndex.current = indexVersion;
      setRevision((n) => n + 1);
    }
  }, [indexVersion, editing]);
  const dragging = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  const suppressClick = useRef(false);
  const [text, setText] = useState(request?.text || "");
  const [namesOnly, setNamesOnly] = useState(!includePath);
  const [partition, setPartition] = useState("");
  const [options, setOptions] = useState<SearchOptions>(emptyOptions);
  const [types, setTypes] = useState<string[]>([]);
  useEffect(() => {
    setTypes((previous) => {
      const next = previous.filter(
        (t) =>
          t === "@folder" || categories.some((c) => t === `category:${c.name}`),
      );
      return next.length === previous.length ? previous : next;
    });
  }, [categories]);
  const [custom, setCustom] = useState("");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [total, setTotal] = useState(0);
  const [groups, setGroups] = useState(0);
  const [missing, setMissing] = useState(0);
  const [unavailable, setUnavailable] = useState<string[]>([]);
  const [revision, setRevision] = useState(0);
  const [preview, setPreview] = useState(false);
  const [selection, setSelection] = useState<Selection>(emptySelection);
  const selectedSnapshot = useRef<{ index: number; hit: SearchHit } | null>(
    null,
  );
  const anchor = useRef(0);
  const cursor = useRef(0);
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    row: SearchHit;
  } | null>(null);
  const [cache, setCache] = useState<Map<number, SearchHit>>(new Map());
  const queryId = useRef<number | null>(null);
  const sequence = useRef(0);
  const [resultVersion, setResultVersion] = useState(0);
  const [range, setRange] = useState<[number, number]>([0, 24]);
  const fetching = useRef(false);
  const viewport = useCallback(
    (a: number, b: number) =>
      setRange((previous) =>
        previous[0] === a && previous[1] === b ? previous : [a, b],
      ),
    [],
  );
  useEffect(() => {
    if (active) input.current?.focus();
    else setMenu(null);
  }, [active, request]);
  useEffect(() => {
    if (request) {
      setText(request.text);
      setRevision((n) => n + 1);
    }
  }, [request]);
  const extensionKey = JSON.stringify([
    ...new Set([
      ...categories
        .filter((c) => types.includes(`category:${c.name}`))
        .flatMap((c) => c.extensions),
      ...custom
        .split(/[,;]/)
        .map((s) => s.trim())
        .filter(Boolean),
    ]),
  ]);
  const scopeKey = JSON.stringify(context);
  useEffect(() => {
    const token = ++sequence.current;
    let id: number | undefined;
    queryId.current = null;
    setBusy(true);
    setError("");
    setTotal(0);
    setCache(new Map());
    setSelection(emptySelection());
    selectedSnapshot.current = null;
    setMenu(null);
    setEditing(null);
    setUnavailable([]);
    setGroups(0);
    setMissing(0);
    fetching.current = false;
    async function run() {
      const extensions = JSON.parse(extensionKey) as string[];
      const folder = types.includes("@folder");
      const queryOptions = {
        ...options,
        extensions,
        kind: folder && !extensions.length ? "folder" : null,
        include_folders: folder && !!extensions.length,
        scope:
          context && partition === "@folder"
            ? {
                partition_id: context.partition,
                entry: context.entry,
                recursive: true,
              }
            : null,
      };
      const start = await indexRequest({
        search_start: {
          text,
          names_only: namesOnly,
          partitions: partition.startsWith("@")
            ? context
              ? [context.partition]
              : []
            : partition
              ? [partition]
              : [],
          options: queryOptions,
        },
      });
      if (start.kind !== "search_started") throw Error("搜索响应无效");
      id = start.data;
      if (token !== sequence.current) {
        await indexRequest({ search_cancel: { id } });
        return;
      }
      queryId.current = id;
      while (token === sequence.current) {
        const reply = await indexRequest({
          search_batch: { id, offset: 0, limit: 32 },
        });
        if (token !== sequence.current) return;
        if (reply.kind !== "search_page") throw Error("搜索分页响应无效");
        const page = reply.data;
        if (page.error) throw Error(page.error);
        if (!page.done) {
          await new Promise((r) => window.setTimeout(r, 100));
          continue;
        }
        setTotal(page.total);
        setGroups(page.groups);
        setMissing(page.missing);
        setUnavailable(page.unavailable);
        setCache(new Map(page.rows.map((row, i) => [i, row])));
        setBusy(false);
        setResultVersion((n) => n + 1);
        return;
      }
    }
    const timer = window.setTimeout(() => {
      void run().catch((e) => {
        if (token === sequence.current) {
          setError(String(e).replace(/^Error: /, ""));
          setBusy(false);
        }
      });
    }, 150);
    return () => {
      sequence.current++;
      window.clearTimeout(timer);
      if (id !== undefined)
        void indexRequest({ search_cancel: { id } }).catch(() => {});
    };
  }, [
    text,
    namesOnly,
    partition,
    options,
    extensionKey,
    types.includes("@folder"),
    scopeKey,
    revision,
  ]);
  useEffect(() => {
    const id = queryId.current;
    const token = sequence.current;
    if (id === null || busy || !total || fetching.current) return;
    let start = range[0];
    const end = Math.min(total, range[1]);
    while (start < end && cache.has(start)) start++;
    if (start >= end) return;
    fetching.current = true;
    let live = true;
    void indexRequest({
      search_batch: { id, offset: start, limit: Math.min(100, end - start) },
    })
      .then((reply) => {
        if (!live || token !== sequence.current) return;
        if (reply.kind !== "search_page") throw Error("搜索分页响应无效");
        if (reply.data.error) throw Error(reply.data.error);
        if (!reply.data.rows.length) throw Error("搜索结果页为空，请重新搜索");
        setCache((previous) => {
          const next = new Map(
            [...previous].filter(([i]) => i >= range[0] - 100 && i < end + 100),
          );
          reply.data.rows.forEach((row, i) => next.set(start + i, row));
          return next;
        });
      })
      .catch((e) => {
        if (live && token === sequence.current) setError(String(e));
      })
      .finally(() => {
        if (token === sequence.current) fetching.current = false;
      });
    return () => {
      live = false;
      fetching.current = false;
    };
  }, [cache, range, total, busy, resultVersion]);
  const selected = (i: number) =>
    !!selection.range && i >= selection.range[0] && i <= selection.range[1]
      ? !selection.toggles.has(i)
      : selection.toggles.has(i);
  const count =
    (selection.range ? selection.range[1] - selection.range[0] + 1 : 0) +
    [...selection.toggles].reduce(
      (n, i) =>
        n +
        (selection.range && i >= selection.range[0] && i <= selection.range[1]
          ? -1
          : 1),
      0,
    );
  const loadedSelected =
    count === 1 ? [...cache].find(([i]) => selected(i))?.[1] : undefined;
  if (loadedSelected) {
    const index = [...cache].find(([, hit]) => hit === loadedSelected)![0];
    selectedSnapshot.current = { index, hit: loadedSelected };
  }
  const selectedHit =
    loadedSelected ||
    (count === 1 &&
    selectedSnapshot.current &&
    selected(selectedSnapshot.current.index)
      ? selectedSnapshot.current.hit
      : undefined);
  const selectedRow = selectedHit ? currentHit(selectedHit) : undefined;
  function choose(i: number, shift: boolean, ctrl: boolean) {
    cursor.current = i;
    if (shift) {
      setSelection({
        range: [Math.min(anchor.current, i), Math.max(anchor.current, i)],
        toggles: new Set(),
      });
    } else if (ctrl) {
      setSelection((s) => {
        const toggles = new Set(s.toggles);
        if (toggles.has(i)) toggles.delete(i);
        else toggles.add(i);
        return { ...s, toggles };
      });
      anchor.current = i;
    } else {
      anchor.current = i;
      setSelection({ range: [i, i], toggles: new Set() });
    }
  }
  function cancel() {
    const token = ++sequence.current;
    if (queryId.current !== null)
      void indexRequest({ search_cancel: { id: queryId.current } }).catch(
        (e) => {
          if (token === sequence.current)
            setError(`取消请求失败：${String(e).replace(/^Error: /, "")}`);
        },
      );
    queryId.current = null;
    setBusy(false);
    setError("搜索已取消");
  }
  const knownSelected = [...cache]
    .filter(([i]) => selected(i))
    .map(([, row]) => currentHit(row));
  const previewRows =
    count === 1 && selectedRow
      ? [selectedRow]
      : knownSelected.length === count
        ? knownSelected
        : [];
  const previewFirst = previewRows[0];
  const online =
    count > 0 &&
    (count === 1
      ? selectedRow?.connection === "online"
      : knownSelected.every((r) => r.connection === "online"));
  const sameParent =
    knownSelected.length < 2 ||
    knownSelected.every(
      (r) =>
        r.filesystem_id === knownSelected[0].filesystem_id &&
        r.file.parent.join(",") === knownSelected[0].file.parent.join(","),
    );
  function operationSelection(override?: number): SelectionInput | null {
    return queryId.current === null
      ? null
      : {
          origin: {
            kind: "search",
            id: queryId.current,
            range:
              override === undefined ? selection.range : [override, override],
          },
          entries: [],
          positions: override === undefined ? [...selection.toggles] : [],
        };
  }
  function perform(action: FileAction, override?: number) {
    const selected = operationSelection(override);
    if (selected)
      operations.run(selected, action, () => setRevision((n) => n + 1));
  }
  function open(row: SearchHit, index?: number) {
    if (row.file.kind === "folder") onLocate?.(row, true);
    else perform("open", index);
  }
  function rename() {
    if (online && count === 1) {
      const index = [...cache].find(([i]) => selected(i))?.[0];
      if (index !== undefined) setEditing(index);
    }
  }
  function menuAction(action: MenuAction) {
    if (action === "open") {
      if (selectedRow) open(selectedRow);
      return;
    }
    if (action === "browse_parent") {
      if (selectedRow) onLocate?.(selectedRow);
      return;
    }
    if (action === "rename") {
      rename();
      return;
    }
    if (action === "paste") return;
    if (action === "properties" && (!online || count !== 1)) {
      setPreview(true);
      return;
    }
    if (action === "more") {
      if (menu) perform({ more: { x: menu.x, y: menu.y } });
      return;
    }
    perform(action);
  }
  return (
    <div className="assets-app search-app">
      {onBack && <button onClick={onBack}>硬盘管理</button>}
      <div className="search-toolbar">
        <input
          ref={input}
          autoFocus
          type="search"
          aria-label="搜索文件或路径"
          placeholder="搜索文件名或路径…"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <select
          aria-label="分区范围"
          value={partition}
          onChange={(e) => setPartition(e.target.value)}
        >
          <option value="">全部已登记分区</option>
          {context && (
            <>
              <option value="@partition">原浏览分区</option>
              <option value="@folder">{context.name}及子目录</option>
            </>
          )}
          {partitions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <SearchFilters
          options={options}
          setOptions={setOptions}
          categories={categories}
          types={types}
          setTypes={setTypes}
          custom={custom}
          setCustom={setCustom}
        />
        <details className="view-menu">
          <summary>视图</summary>
          <div>
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
                checked={namesOnly}
                onChange={(e) => setNamesOnly(e.target.checked)}
              />
              仅文件名
            </label>
          </div>
        </details>
        <button onClick={busy ? cancel : () => setRevision((n) => n + 1)}>
          {busy ? "取消搜索" : "重新搜索"}
        </button>
        <span className="search-summary" role="status">
          {busy
            ? "正在搜索…"
            : error
              ? "搜索失败"
              : `${total.toLocaleString()} 条结果`}
        </span>
      </div>
      {error && (
        <p role="alert" className="asset-error">
          {error}
        </p>
      )}
      {unavailable.length > 0 && (
        <details className="search-unavailable">
          <summary>{unavailable.length} 个分区未参与搜索</summary>
          {unavailable.map((v, i) => (
            <p key={i}>{v}</p>
          ))}
        </details>
      )}
      <div className="search-body" onClick={() => setMenu(null)}>
        <VirtualResults
          sort={options.sort}
          descending={options.descending}
          onSort={(sort) =>
            setOptions({
              ...options,
              sort,
              descending: options.sort === sort ? !options.descending : false,
            })
          }
          total={total}
          revision={resultVersion}
          onRange={viewport}
          onKeyDown={(e) => {
            if (
              fileKey(e, {
                copy: () => perform("copy"),
                cut: () => perform("cut"),
                rename,
                remove: (permanent) =>
                  perform(permanent ? "delete" : "recycle"),
                properties: () => menuAction("properties"),
                menu: () => {
                  if (selectedRow) {
                    const rect = e.currentTarget.getBoundingClientRect();
                    setMenu({
                      x: rect.left + 20,
                      y: rect.top + 45,
                      row: selectedRow,
                    });
                  }
                },
              })
            )
              return;
            if (e.ctrlKey && e.key.toLowerCase() === "a") {
              e.preventDefault();
              setSelection({
                range: total ? [0, total - 1] : null,
                toggles: new Set(),
              });
            }
            if (e.key === "Escape") setMenu(null);
            if ((e.key === "ArrowDown" || e.key === "ArrowUp") && total) {
              e.preventDefault();
              const i = Math.max(
                0,
                Math.min(
                  total - 1,
                  cursor.current + (e.key === "ArrowDown" ? 1 : -1),
                ),
              );
              choose(i, e.shiftKey, false);
              const height = e.currentTarget.clientHeight || 500;
              const scale = virtualScale(total, height);
              const offset = (i * 64) / scale;
              const virtualTop = e.currentTarget.scrollTop * scale;
              if (i * 64 < virtualTop || i * 64 > virtualTop + height - 100)
                e.currentTarget.scrollTop = offset;
            }
            if (e.key === "Enter" && selectedRow) open(selectedRow);
          }}
          row={(i) => {
            const cached = cache.get(i);
            const r = cached ? currentHit(cached) : undefined;
            return (
              <tr
                key={i}
                aria-rowindex={i + 2}
                aria-selected={selected(i)}
                className="virtual-result-row"
                onClick={(e) => {
                  if (suppressClick.current) {
                    suppressClick.current = false;
                    return;
                  }
                  choose(i, e.shiftKey, e.ctrlKey);
                }}
                onDoubleClick={() => {
                  if (r) open(r, i);
                }}
                onPointerDown={(e) => {
                  if (e.button === 0) {
                    suppressClick.current = false;
                    dragging.current = {
                      x: e.clientX,
                      y: e.clientY,
                      started: false,
                    };
                  }
                }}
                onPointerMove={(e) => {
                  const start = dragging.current;
                  if (
                    !r ||
                    !start ||
                    start.started ||
                    e.buttons !== 1 ||
                    Math.hypot(e.clientX - start.x, e.clientY - start.y) < 8
                  )
                    return;
                  start.started = true;
                  suppressClick.current = true;
                  e.preventDefault();
                  const current = selected(i);
                  if (!current) choose(i, false, false);
                  perform("drag", current ? undefined : i);
                }}
                onPointerUp={() => {
                  dragging.current = null;
                }}
                onDragStart={(e) => e.preventDefault()}
                onContextMenu={(e) => {
                  if (!r) return;
                  e.preventDefault();
                  e.currentTarget.tabIndex = -1;
                  e.currentTarget.focus();
                  if (!selected(i)) choose(i, false, false);
                  setMenu({
                    x: Math.min(e.clientX, window.innerWidth - 270),
                    y: Math.min(e.clientY, window.innerHeight - 60),
                    row: r,
                  });
                }}
              >
                {!r ? (
                  <td colSpan={7}>正在读取…</td>
                ) : (
                  <>
                    <td title={r.file.name}>
                      <span
                        className={`file-mark ${r.file.kind}`}
                        aria-hidden="true"
                      />
                      {editing === i ? (
                        <RenameInput
                          name={r.file.name}
                          folder={r.file.kind === "folder"}
                          onCancel={() => setEditing(null)}
                          onSubmit={(name) => {
                            setEditing(null);
                            perform({ rename: name }, i);
                          }}
                        />
                      ) : (
                        r.file.name
                      )}
                      {r.hard_link && <small>硬链接</small>}
                    </td>
                    <td title={r.file.path}>
                      {r.connection === "online" ? r.mounts[0] || "" : ""}
                      {r.file.path}
                    </td>
                    <td>{r.partition_name}</td>
                    <td
                      className="result-size"
                      title={
                        r.file.size === null
                          ? undefined
                          : `${BigInt(r.file.size).toLocaleString()} B`
                      }
                    >
                      {r.file.kind === "folder"
                        ? "—"
                        : r.file.size === null
                          ? "未知"
                          : fileSize(r.file.size)}
                    </td>
                    <td>{fileTime(r.file.modified)}</td>
                    <td>
                      {connectionLabel[r.connection] || "状态未确认"}
                      {r.partial ? (
                        <small>未完成记录</small>
                      ) : r.needs_validation ? (
                        <small>索引待校验</small>
                      ) : null}
                      <small>
                        {r.indexed_at == null
                          ? "索引时间未知"
                          : new Date(r.indexed_at * 1000).toLocaleString()}
                      </small>
                    </td>
                    <td>{r.duplicate_group || "—"}</td>
                  </>
                )}
              </tr>
            );
          }}
        />
        {preview && (
          <FileDetails
            selectionCount={count}
            rows={previewRows.map((r) => r.file)}
            partition={
              previewFirst &&
              previewRows.every(
                (r) => r.partition_id === previewFirst.partition_id,
              )
                ? previewFirst.partition_name
                : ""
            }
            connection={
              previewFirst &&
              previewRows.every((r) => r.connection === previewFirst.connection)
                ? previewFirst.connection
                : null
            }
            indexedAt={
              previewFirst &&
              previewRows.every((r) => r.indexed_at === previewFirst.indexed_at)
                ? previewFirst.indexed_at
                : null
            }
          />
        )}
      </div>
      {!busy && !error && !total && (
        <p className="search-empty">没有匹配的文件</p>
      )}
      <footer className="browse-status">
        <span>
          {count
            ? `已选择 ${count.toLocaleString()} 项`
            : groups
              ? `${groups.toLocaleString()} 个重复候选组`
              : ""}
        </span>
        <span>
          {missing ? `${missing.toLocaleString()} 项缺少查重元数据` : ""}
        </span>
      </footer>
      {menu && (
        <FileContextMenu
          x={menu.x}
          y={menu.y}
          count={count}
          online={online}
          sameParent={sameParent}
          singleFile={count === 1 && selectedRow?.file.kind === "file"}
          folder={count === 1 && selectedRow?.file.kind === "folder"}
          onAction={menuAction}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
