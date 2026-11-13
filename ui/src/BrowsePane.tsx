import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Asset } from "./asset-api";
import {
  connectionLabel,
  readDirectory,
  type DirectoryPage,
  type DirectorySort,
  type DirectorySortField,
} from "./browse-api";
import DirectoryTree from "./DirectoryTree";
import FileList from "./FileList";
import { fileViewOptions, type FileView } from "./file-view";
import FileDetails from "./FileDetails";
import FileContextMenu from "./FileContextMenu";
import { FileStatusBar, useFileOperations } from "./FileOperations";
import {
  fileKey,
  fileEditingKey,
  fileSelectionLimit,
  maxFileSelection,
  useFileCommands,
  type FileCommandContext,
  type MenuAction,
  type FileAction,
  type SelectionInput,
  type EntryRef,
} from "./file-operations";
import type { FileRow } from "./browse-api";
import type { FileFilter } from "./search-options";
import { directoryColumns, sortCommands } from "./file-columns";
import {
  eventShortcut,
  workspaceCommand,
  workspaceShortcuts,
} from "./workspace-shortcuts";

export type BrowseLocation = {
  partition: string;
  filesystem: string;
  entry: number[];
};

export default function BrowsePane({
  id,
  asset,
  assets,
  initialParent,
  historyPage,
  history,
  onHistory,
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
  view,
  onView,
  filter,
}: {
  id: number;
  asset?: Asset;
  assets: Asset[];
  initialParent: number[] | null;
  historyPage?: DirectoryPage;
  history?: { entries: BrowseLocation[]; cursor: number };
  onHistory?: (
    cursor: number,
    target: BrowseLocation,
    page: DirectoryPage,
  ) => void;
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
  view: FileView;
  onView: (view: FileView) => void;
  emptyState?: ReactNode;
  filter?: FileFilter;
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
  const [sort, setSort] = useState<DirectorySort>();
  const appliedSort = useRef<DirectorySort>(undefined);
  const [tree, setTree] = useState(true);
  const [preview, setPreview] = useState(false);
  const [breadcrumb, setBreadcrumb] = useState(true);
  const [searchText, setSearchText] = useState("");
  const composing = useRef(false);
  const request = useRef(0);
  const currentPage = useRef(page);
  const usedHistoryPage = useRef<DirectoryPage>(undefined);
  const partition = asset?.registration.partition_id;
  const filesystem = asset?.registration.filesystem_id;
  useEffect(() => {
    if (page && partition) onLocation?.(id, page, partition, active);
  }, [id, page, partition, active, onLocation]);
  currentPage.current = page;
  const parentKey = JSON.stringify(parent);
  const filterKey = filter ? JSON.stringify(filter) : "";
  const appliedFilter = useRef(filterKey);
  useEffect(() => {
    if (!partition || !filesystem) return;
    const token = ++request.current;
    const rule = filterKey ? (JSON.parse(filterKey) as FileFilter) : undefined;
    if (appliedFilter.current !== filterKey) {
      setPage(null);
      setSelection([]);
      setAllSelected(false);
      setEditing(null);
      setMenu(null);
      setNotice("");
    }
    setBusy(true);
    setError("");
    async function load() {
      let result: DirectoryPage;
      try {
        if (historyPage && usedHistoryPage.current !== historyPage) {
          result = historyPage;
          usedHistoryPage.current = historyPage;
        } else
          result = await readDirectory({
            partition_id: partition!,
            filesystem_id: filesystem!,
            parent: JSON.parse(parentKey),
            ...(sort && { sort }),
            ...(rule && { filter: rule }),
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
              ...(sort && { sort }),
              ...(rule && { filter: rule }),
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
      appliedSort.current = sort;
      appliedFilter.current = filterKey;
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
        const message = String(e).replace(/^Error: /, "");
        if (message !== "尚无目录索引" && message !== "目录索引尚未完成") {
          setError(message);
        }
        setBusy(false);
      }
    });
    return () => {
      request.current++;
    };
  }, [partition, filesystem, parentKey, refresh, sort, filterKey, historyPage]);
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
  async function visitHistory(delta: number) {
    if (!history || !onHistory || busy) return;
    const cursor = history.cursor + delta;
    const target = history.entries[cursor];
    if (!target) return;
    const token = ++request.current;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (
        !assets.some(
          ({ registration }) =>
            registration.partition_id === target.partition &&
            registration.filesystem_id === target.filesystem,
        )
      )
        throw Error("分区记录已不可用");
      const result = await readDirectory({
        partition_id: target.partition,
        filesystem_id: target.filesystem,
        parent: target.entry,
        ...(target.partition === partition && sort && { sort }),
        ...(filter && { filter }),
      });
      if (token !== request.current) return;
      navigate(target.entry);
      onHistory(cursor, target, result);
    } catch (e) {
      if (token !== request.current) return;
      setError(`无法打开历史目录：${String(e).replace(/^Error: /, "")}`);
      setBusy(false);
    }
  }
  function reload() {
    setParent(page?.location || parent);
    setRefresh((n) => n + 1);
  }
  const count = allSelected
    ? Math.max(0, (page?.total ?? page?.entries.length ?? 0) - selection.length)
    : selection.length;
  const selectedIds = new Set(selection);
  const selectedRows =
    page?.entries.filter(
      (row) => allSelected !== selectedIds.has(row.entry.join(",")),
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
            ...(!override && allSelected && filter && { filter }),
          },
          entries:
            override ?? selection.map((key) => key.split(",").map(Number)),
          positions: [],
        }
      : null;
  }
  function perform(action: FileAction, override?: number[][]) {
    if (!override && count > maxFileSelection) return;
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
  function selectAll() {
    setSelection([]);
    setAllSelected(true);
  }
  const up =
    page && page.ancestors.length > 1
      ? page.ancestors[page.ancestors.length - 2]
      : undefined;
  function goUp() {
    if (up) navigate(up.entry);
  }
  const dispatch = useFileCommands(
    facts,
    menuAction,
    active,
    onCommands,
    [
      ...fileViewOptions(view, onView),
      { label: "目录树", checked: tree, run: () => setTree((v) => !v) },
      {
        ...workspaceCommand("preview", () => setPreview((v) => !v)),
        checked: preview,
      },
      {
        label: "面包屑",
        checked: breadcrumb,
        run: () => setBreadcrumb((v) => !v),
      },
    ],
    asset && !busy && editing === null
      ? () => {
          setNotice("");
          reload();
        }
      : undefined,
    onCommands && {
      selection: [
        workspaceCommand(
          "selectAll",
          selectAll,
          busy || !!error || editing !== null || !page?.total,
        ),
        {
          label: "反选",
          disabled:
            busy ||
            !!error ||
            editing !== null ||
            !(page?.total ?? page?.entries.length),
          run: () => setAllSelected((all) => !all),
        },
      ],
      sorting: sortCommands(
        directoryColumns,
        sort?.field ?? "name",
        sort?.descending ?? false,
        (field, descending) =>
          changeSort(field as DirectorySortField, descending),
      ),
      navigation: [
        workspaceCommand("up", goUp, !up || busy || editing !== null),
        workspaceCommand(
          "back",
          () => void visitHistory(-1),
          busy || editing !== null || !history || history.cursor === 0,
        ),
        workspaceCommand(
          "forward",
          () => void visitHistory(1),
          busy ||
            editing !== null ||
            !history ||
            history.cursor + 1 >= history.entries.length,
        ),
      ],
    },
  );
  function fileKeys(e: React.KeyboardEvent) {
    return fileKey(e, dispatch, () => {
      const rect = (e.target as HTMLElement).getBoundingClientRect();
      showMenu(rect.left + 20, rect.bottom, count === 0);
    });
  }
  function sortBy(field: DirectorySortField) {
    changeSort(field, (sort?.field ?? "name") === field && !sort?.descending);
  }
  function changeSort(field: DirectorySortField, descending: boolean) {
    request.current++;
    setBusy(true);
    setSelection([]);
    setAllSelected(false);
    setEditing(null);
    setMenu(null);
    setSort({
      field,
      descending,
    });
  }
  async function more() {
    if (
      !page ||
      page.next === null ||
      busy ||
      appliedSort.current !== sort ||
      appliedFilter.current !== filterKey
    )
      return;
    const token = request.current;
    setBusy(true);
    setError("");
    setNotice("");
    await readDirectory({
      partition_id: partition!,
      filesystem_id: filesystem!,
      parent: page.location,
      generation: page.generation,
      revision: page.revision,
      offset: page.next,
      ...(sort && { sort }),
      ...(filter && { filter }),
    }).then(
      (value) => {
        if (token === request.current) {
          if (
            page.filesystem_id === value.filesystem_id &&
            page.generation === value.generation &&
            page.revision === value.revision &&
            page.location.join(",") === value.location.join(",")
          ) {
            setPage({ ...value, entries: [...page.entries, ...value.entries] });
          } else setNotice("目录索引已更新，请刷新");
          setBusy(false);
        }
      },
      (e) => {
        if (token === request.current) {
          const message = String(e).replace(/^Error: /, "");
          if (message === "目录索引已更新，请刷新") setNotice(message);
          else setError(message);
          setBusy(false);
        }
      },
    );
  }
  const partitionTree = tree && (
    <nav className="directory-tree" aria-label="目录树">
      {assets.map(({ registration }) => {
        const current = registration.partition_id === partition;
        return (
          <DirectoryTree
            key={registration.partition_id}
            partition={registration.partition_id}
            filesystem={registration.filesystem_id}
            name={registration.name}
            activePartition={current}
            canExpand={page !== null}
            selected={current ? (page?.location ?? null) : null}
            ancestors={current ? page?.ancestors : undefined}
            version={
              current && page
                ? `${page.generation}:${page.revision}`
                : undefined
            }
            onNavigate={
              current ? navigate : () => onChoose(registration.partition_id)
            }
          />
        );
      })}
    </nav>
  );
  return (
    <div
      className="browse-pane"
      onKeyDown={(e) => {
        if (
          !e.repeat &&
          !e.metaKey &&
          !fileEditingKey(e) &&
          eventShortcut(e.nativeEvent) === workspaceShortcuts.up[1] &&
          up &&
          !busy &&
          editing === null
        ) {
          e.preventDefault();
          goUp();
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
          {partitionTree}
          <div className="directory-main">
            <div className="search-empty">
              {emptyState ?? <p>选择已登记分区以浏览文件</p>}
            </div>
          </div>
        </div>
      ) : (
        <>
          <div className="browse-body">
            {partitionTree}
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
                  <button
                    title={asset.registration.name}
                    onClick={() => navigate(null)}
                  >
                    {asset.registration.name}
                  </button>
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
              {page && (
                <FileList
                  active={active}
                  view={view}
                  sort={sort}
                  onSort={sortBy}
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
                    if (!busy && !error && editing === null) selectAll();
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
                  next={appliedSort.current === sort ? page.next : null}
                  busy={busy}
                  onMore={() => void more()}
                />
              )}
              {!page && !error && (
                <p className="directory-loading" role="status">
                  正在加载目录索引…
                </p>
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
          <FileStatusBar>
            <span role="status">
              {busy && page
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
            </span>
            {count > maxFileSelection && (
              <span role="status">{fileSelectionLimit}</span>
            )}
          </FileStatusBar>
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
      {!asset && <FileStatusBar />}
    </div>
  );
}
