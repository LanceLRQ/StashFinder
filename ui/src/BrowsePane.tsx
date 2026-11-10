import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Asset } from "./asset-api";
import {
  connectionLabel,
  readDirectory,
  type DirectoryPage,
} from "./browse-api";
import DirectoryTree from "./DirectoryTree";
import FileList from "./FileList";
import FileDetails from "./FileDetails";
import FileContextMenu from "./FileContextMenu";
import { useFileOperations } from "./FileOperations";
import {
  fileKey,
  useFileCommands,
  type FileCommandContext,
  type MenuAction,
  type FileAction,
  type SelectionInput,
  type EntryRef,
} from "./file-operations";
import type { FileRow } from "./browse-api";

export default function BrowsePane({
  id,
  asset,
  assets,
  initialParent,
  focusEntry,
  onChoose,
  onSearch,
  onTitle,
  onLocation,
  onLocate,
  indexVersion = "",
  automatic = false,
  active = true,
  emptyState,
  onCommands,
}: {
  id: number;
  asset?: Asset;
  assets: Asset[];
  initialParent: number[] | null;
  focusEntry?: number[];
  onChoose: (partition: string) => void;
  onSearch: (text: string) => void;
  onTitle: (id: number, title: string) => void;
  onLocation?: (
    id: number,
    page: DirectoryPage,
    partition: string,
    active: boolean,
  ) => void;
  onLocate?: (row: FileRow) => void;
  indexVersion?: string;
  automatic?: boolean;
  active?: boolean;
  onCommands?: (context: FileCommandContext | null) => void;
  emptyState?: ReactNode;
}) {
  const [parent, setParent] = useState(initialParent);
  const operations = useFileOperations();
  const [allSelected, setAllSelected] = useState(false);
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    blank: boolean;
  } | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  useEffect(() => {
    if (!active) setMenu(null);
  }, [active]);
  const lastIndex = useRef(indexVersion);
  useEffect(() => {
    if (lastIndex.current !== indexVersion && editing === null) {
      lastIndex.current = indexVersion;
      setRefresh((n) => n + 1);
    }
  }, [indexVersion, editing]);
  const [page, setPage] = useState<DirectoryPage | null>(null);
  const [selection, setSelection] = useState<string[]>(
    focusEntry ? [focusEntry.join(",")] : [],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [tree, setTree] = useState(true);
  const [preview, setPreview] = useState(false);
  const [breadcrumb, setBreadcrumb] = useState(true);
  const [searchText, setSearchText] = useState("");
  const composing = useRef(false);
  const request = useRef(0);
  const currentPage = useRef(page);
  const partition = asset?.registration.partition_id;
  const filesystem = asset?.registration.filesystem_id;
  useEffect(() => {
    if (page && partition) onLocation?.(id, page, partition, active);
  }, [id, page, partition, active, onLocation]);
  currentPage.current = page;
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
      const previous = currentPage.current;
      if (
        previous &&
        (previous.generation !== result.generation ||
          previous.revision !== result.revision)
      ) {
        setSelection([]);
        setAllSelected(false);
        setNotice("");
      }
      setPage(result);
      if (!previous)
        setSelection((ids) =>
          ids.filter((key) =>
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
    setAllSelected(false);
    setMenu(null);
    setEditing(null);
    setSelection([]);
    setNotice("");
    setParent(next);
    setRefresh((n) => n + 1);
  }
  function reload() {
    setParent(page?.location || parent);
    setRefresh((n) => n + 1);
  }
  const count = allSelected
    ? Math.max(0, (page?.total ?? page?.entries.length ?? 0) - selection.length)
    : selection.length;
  const selectedRows =
    page?.entries.filter((row) =>
      allSelected
        ? !selection.includes(row.entry.join(","))
        : selection.includes(row.entry.join(",")),
    ) ?? [];
  const online =
    asset?.connection === "online" && asset.registration.state === "ready";
  function context(): EntryRef | null {
    return page && partition && filesystem
      ? {
          partition_id: partition,
          filesystem_id: filesystem,
          generation: page.generation,
          revision: page.revision,
          entry: page.location,
        }
      : null;
  }
  function input(override?: number[][]): SelectionInput | null {
    const ref = context();
    return ref
      ? {
          origin: {
            kind: "directory",
            context: ref,
            all: override ? false : allSelected,
          },
          entries:
            override ?? selection.map((key) => key.split(",").map(Number)),
          positions: [],
        }
      : null;
  }
  function perform(action: FileAction, override?: number[][]) {
    const selected = input(override);
    if (selected)
      operations.run(selected, action, () => {
        setNotice(
          automatic
            ? "文件操作已结束，索引更新中"
            : "文件操作已结束，索引待更新",
        );
        reload();
      });
  }
  function open(row: FileRow) {
    if (row.kind === "folder") navigate(row.entry);
    else perform("open", [row.entry]);
  }
  function rename() {
    if (online && count === 1 && selectedRows[0])
      setEditing(selectedRows[0].entry.join(","));
  }
  function showMenu(x: number, y: number, blank = false) {
    setMenu({ x, y, blank });
  }
  function menuAction(action: MenuAction) {
    if (action === "open") {
      if (selectedRows[0]) open(selectedRows[0]);
      return;
    }
    if (action === "browse_parent") {
      if (selectedRows[0]) onLocate?.(selectedRows[0]);
      return;
    }
    if (action === "rename") {
      rename();
      return;
    }
    if (action === "paste") {
      const target = context();
      if (target)
        operations.paste(target, () => {
          setNotice(
            automatic
              ? "文件操作已结束，索引更新中"
              : "文件操作已结束，索引待更新",
          );
          reload();
        });
      return;
    }
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
  const facts = {
    count,
    online: !!online,
    singleFile: count === 1 && selectedRows[0]?.kind === "file",
    sameParent: true,
    folder: count === 1 && selectedRows[0]?.kind === "folder",
    paste:
      !!online && !!page && !page.partial && !asset?.error && !busy && !error,
    blocked: editing !== null,
  };
  const dispatch = useFileCommands(
    facts,
    menuAction,
    active,
    onCommands,
    [
      { label: "目录树", checked: tree, run: () => setTree((v) => !v) },
      { label: "预览窗格", checked: preview, run: () => setPreview((v) => !v) },
      {
        label: "面包屑",
        checked: breadcrumb,
        run: () => setBreadcrumb((v) => !v),
      },
    ],
    asset && !busy && editing === null ? reload : undefined,
  );
  function fileKeys(e: React.KeyboardEvent) {
    return fileKey(e, dispatch, () => {
      const rect = (e.target as HTMLElement).getBoundingClientRect();
      showMenu(rect.left + 20, rect.bottom, count === 0);
    });
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
  const partitionSelect = (
    <select
      aria-label="浏览分区"
      value=""
      onChange={(e) => {
        const next = e.target.value;
        if (next === partition) navigate(null);
        else onChoose(next);
      }}
    >
      <option value="" disabled>
        {asset?.registration.name ?? "选择分区"}
      </option>
      {assets.map(({ registration }) => (
        <option
          key={registration.partition_id}
          value={registration.partition_id}
        >
          {registration.name}
          {registration.partition_id === partition ? "（根目录）" : ""}
        </option>
      ))}
    </select>
  );
  return (
    <div
      className="browse-pane"
      onKeyDown={(e) => {
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
      </div>
      {!asset ? (
        <div className="browse-body">
          {tree && <nav className="directory-tree" aria-label="目录树" />}
          <div className="directory-main">
            {breadcrumb && assets.length > 0 && (
              <nav className="breadcrumbs" aria-label="目录位置">
                {partitionSelect}
              </nav>
            )}
            <div className="search-empty">
              {emptyState ?? <p>选择已登记分区以浏览文件</p>}
            </div>
          </div>
        </div>
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
                {assets.map(({ registration }) =>
                  registration.partition_id === partition ? (
                    <DirectoryTree
                      key={partition}
                      partition={partition!}
                      filesystem={filesystem!}
                      name={registration.name}
                      selected={page?.location || null}
                      ancestors={page?.ancestors}
                      version={
                        page ? `${page.generation}:${page.revision}` : ""
                      }
                      onNavigate={navigate}
                    />
                  ) : (
                    <div
                      className="tree-row other-partition"
                      key={registration.partition_id}
                    >
                      <button
                        className="tree-label"
                        title={registration.name}
                        onClick={() => onChoose(registration.partition_id)}
                      >
                        <span className="file-mark folder" aria-hidden="true" />
                        {registration.name}
                      </button>
                    </div>
                  ),
                )}
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
                  {partitionSelect}
                  {page?.ancestors.slice(1).map((a) => (
                    <span key={a.entry.join(",")}>
                      <span aria-hidden="true"> / </span>
                      <button title={a.name} onClick={() => navigate(a.entry)}>
                        {a.name}
                      </button>
                    </span>
                  ))}
                </nav>
              )}
              {page && (
                <FileList
                  active={active}
                  key={page.location.join(",")}
                  rows={page.entries}
                  context={context() || undefined}
                  connection={asset.connection}
                  selection={selection}
                  allSelected={allSelected}
                  onSelect={(ids, all = false) => {
                    setSelection(ids);
                    setAllSelected(all);
                  }}
                  onSelectAll={() => {
                    setSelection([]);
                    setAllSelected(true);
                  }}
                  onContext={(_row, x, y) => showMenu(x, y)}
                  onBlankContext={(x, y) => {
                    setSelection([]);
                    setAllSelected(false);
                    showMenu(x, y, true);
                  }}
                  onFileKey={fileKeys}
                  onDrag={(row, current) => {
                    if (!current) {
                      setSelection([row.entry.join(",")]);
                      setAllSelected(false);
                    }
                    perform("drag", current ? undefined : [row.entry]);
                  }}
                  editing={
                    editing
                      ? {
                          entry: editing,
                          onCancel: () => setEditing(null),
                          onSubmit: (name) => {
                            const key = editing;
                            setEditing(null);
                            perform({ rename: name }, [
                              key.split(",").map(Number),
                            ]);
                          },
                        }
                      : undefined
                  }
                  onOpen={open}
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
                active={active}
                selectionCount={count}
                rows={count === 1 ? selectedRows : []}
                partition={asset.registration.name}
                connection={asset.connection}
                indexedAt={page?.indexed_at ?? null}
                reference={
                  page && count === 1 && selectedRows[0]
                    ? {
                        partition_id: asset.registration.partition_id,
                        filesystem_id: asset.registration.filesystem_id,
                        generation: page.generation,
                        revision: page.revision,
                        entry: selectedRows[0].entry,
                      }
                    : undefined
                }
              />
            )}
          </div>
          <footer className="browse-status">
            <span role="status">
              {busy
                ? "正在读取…"
                : count > 0
                  ? `已选择 ${count.toLocaleString()} 项`
                  : page &&
                    `${(page.total ?? page.entries.length).toLocaleString()} 项`}
            </span>
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
          {active && menu && (
            <FileContextMenu
              x={menu.x}
              y={menu.y}
              count={menu.blank ? 0 : count}
              online={facts.online}
              singleFile={facts.singleFile}
              sameParent={facts.sameParent}
              folder={facts.folder}
              paste={menu.blank && facts.paste}
              blocked={facts.blocked}
              onAction={dispatch}
              onClose={() => {
                setMenu(null);
              }}
            />
          )}
        </>
      )}
    </div>
  );
}
