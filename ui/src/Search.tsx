import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { indexRequest, type SearchHit, type SearchPage } from "./index-api";
import { connectionLabel } from "./browse-api";
import { fileColumnText, searchColumns, sortCommands } from "./file-columns";
import {
  eventShortcut,
  workspaceCommand,
  workspaceShortcuts,
} from "./workspace-shortcuts";
import {
  defaultCategories,
  emptyOptions,
  hasFileFilter,
  noFilter,
  noPresets,
  selectFilter,
  filterPresetEntries,
  type Category,
  type FilterPreset,
  type FilterSelection,
  type SearchContext,
  type SearchOptions,
  type BookmarkedSearch,
} from "./search-options";
import FileDetails from "./FileDetails";
import SearchFilters from "./SearchFilters";
import SearchExport, { type ExportSnapshot } from "./SearchExport";
import VirtualResults, { searchDetailsLayout } from "./VirtualResults";
import { virtualWindow, type ViewportLayout } from "./DetailsViewport";
import { fileViewOptions, thumbnailLayouts, type FileView } from "./file-view";
import { inSpans, rectangleSpans, toggleSpans } from "./marquee";
import FileContextMenu from "./FileContextMenu";
import WorkspaceIcon from "./WorkspaceIcon";
import FileListItem from "./FileListItem";
import type { Asset } from "./asset-api";
import type { PartitionIndexes } from "./task-api";
import { FileStatusBar, useFileOperations } from "./FileOperations";
import {
  fileEditingKey,
  fileKey,
  fileSelectionLimit,
  maxFileSelection,
  useFileCommands,
  type FileCommandContext,
  type MenuAction,
  type FileAction,
  type SelectionInput,
} from "./file-operations";
import "./assets.css";
import "./search.css";
import "./search-advanced.css";
const noIndexes: PartitionIndexes = {};
const inputDelay = 75;
const refreshDelay = 250;
const pause = (ms: number) =>
  new Promise<void>((resolve) => window.setTimeout(resolve, ms));
async function readBatch(
  id: number,
  offset: number,
  limit: number,
  wait = false,
): Promise<SearchPage> {
  const reply = await indexRequest({
    search_batch: { id, offset, limit, ...(wait ? { wait_ms: 1000 } : {}) },
  });
  if (reply.kind !== "search_page") throw Error("搜索分页响应无效");
  if (reply.data.error) throw Error(reply.data.error);
  return reply.data;
}

export default function Search({
  partitions,
  assets,
  onBack,
  includePath = true,
  initialText = "",
  initialView = "details",
  initialPreview = false,
  onlineOnly = false,
  includeHistory = false,
  onTextChange,
  active = true,
  onCommands,
  onLocate,
  categories = defaultCategories,
  initialFilter = noFilter,
  bookmark,
  onBookmark,
  presets = noPresets,
  onSavePresets,
  savingFilters = false,
  context,
  indexes = noIndexes,
}: {
  partitions: { id: string; name: string }[];
  assets?: Asset[];
  onBack?: () => void;
  includePath?: boolean;
  initialText?: string;
  initialView?: FileView;
  initialPreview?: boolean;
  onlineOnly?: boolean;
  includeHistory?: boolean;
  onTextChange?: (text: string) => void;
  active?: boolean;
  onCommands?: (context: FileCommandContext | null) => void;
  onLocate?: (hit: SearchHit, folder?: boolean) => void;
  categories?: Category[];
  initialFilter?: FilterSelection;
  bookmark?: BookmarkedSearch;
  onBookmark?: (query: BookmarkedSearch) => void;
  presets?: FilterPreset[];
  onSavePresets?: (presets: FilterPreset[]) => Promise<boolean>;
  savingFilters?: boolean;
  context?: SearchContext;
  indexes?: PartitionIndexes;
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
  const dragging = useRef<{ x: number; y: number; started: boolean } | null>(
    null,
  );
  const suppressClick = useRef(false);
  const [text, setText] = useState(initialText);
  const [namesOnly, setNamesOnly] = useState(
    bookmark?.names_only ?? !includePath,
  );
  const [partition, setPartition] = useState(
    bookmark?.scope ? "@bookmark" : "",
  );
  const [options, setOptions] = useState<SearchOptions>(() => ({
    ...emptyOptions,
    online_only: onlineOnly,
    include_history: includeHistory,
    ...bookmark?.options,
  }));
  const [filter, setFilter] = useState<FilterSelection>(() => {
    if (!bookmark) return initialFilter;
    const { extensions, include_folders, kind, size, created, modified } =
      bookmark.options;
    return {
      rule: { extensions, include_folders, kind, size, created, modified },
      types: include_folders || kind === "folder" ? ["@folder"] : [],
      custom: extensions.join(","),
    };
  });
  const [filterOpen, setFilterOpen] = useState<"search" | "save" | false>(
    false,
  );
  const filterStatusId = useId();
  const filterActive =
    hasFileFilter(filter.rule) ||
    namesOnly !== !includePath ||
    options.case_sensitive ||
    options.mode !== "contains" ||
    !!options.duplicate.length ||
    options.online_only ||
    options.include_history;
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [total, setTotal] = useState(0);
  const [groups, setGroups] = useState(0);
  const [missing, setMissing] = useState(0);
  const [unavailable, setUnavailable] = useState<string[]>([]);
  const [revision, setRevision] = useState(0);
  const [preview, setPreview] = useState(initialPreview);
  const [exportSnapshot, setExportSnapshot] = useState<ExportSnapshot | null>(
    null,
  );
  const [view, setView] = useState<FileView>(initialView);
  const grid = view === "details" ? undefined : thumbnailLayouts[view];
  const [selection, setSelection] = useState<number[]>([]);
  const selectedSnapshot = useRef<{ index: number; hit: SearchHit } | null>(
    null,
  );
  const anchor = useRef(0);
  const cursor = useRef(0);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [cache, setCache] = useState<Map<number, SearchHit>>(new Map());
  const queryId = useRef<number | null>(null);
  const sequence = useRef(0);
  const [resultVersion, setResultVersion] = useState(0);
  const [range, setRange] = useState<[number, number]>([0, 24]);
  const pendingId = useRef<number | null>(null);
  const replacedId = useRef<number | null>(null);
  const submitPending = useRef<(() => void) | null>(null);
  const previousText = useRef(text);
  const [composing, setComposing] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState("");
  const [refreshWanted, setRefreshWanted] = useState(0);
  const handledRefresh = useRef(0);
  const viewportPosition = useRef({
    logicalTop: 0,
    height: 500,
    lastScroll: 0,
    layout: { ...searchDetailsLayout, columns: 1 },
  });
  const [restore, setRestore] = useState<{ logicalTop: number } | null>(null);
  const onPosition = useCallback(
    (
      logicalTop: number,
      height: number,
      scrolling: boolean,
      layout: ViewportLayout,
    ) => {
      viewportPosition.current = {
        logicalTop,
        height,
        layout,
        lastScroll: scrolling
          ? Date.now()
          : viewportPosition.current.lastScroll,
      };
    },
    [],
  );
  const viewport = useCallback(
    (a: number, b: number) =>
      setRange((previous) =>
        previous[0] === a && previous[1] === b ? previous : [a, b],
      ),
    [],
  );
  useEffect(() => {
    if (active) input.current?.focus();
    else {
      setMenu(null);
      setFilterOpen(false);
    }
  }, [active]);
  const partitionIds = partitions.map((item) => item.id);
  const partitionKey = JSON.stringify([...partitionIds].sort());
  const validContext =
    context && partitionIds.includes(context.partition) ? context : undefined;
  const savedScope = partition === "@bookmark" ? bookmark?.scope : null;
  const scopedAsset = assets?.find(
    (item) => item.registration.partition_id === savedScope?.partition_id,
  );
  const invalidScope =
    !!savedScope &&
    scopedAsset?.registration.filesystem_id !== savedScope.filesystem_id;
  const scope = savedScope
    ? partition
    : partition.startsWith("@")
      ? validContext
        ? partition
        : ""
      : partitionIds.includes(partition)
        ? partition
        : "";
  const scopedPartition =
    savedScope?.partition_id ??
    (scope.startsWith("@") ? validContext?.partition : scope);
  const query = {
    text,
    names_only: namesOnly,
    partitions: scopedPartition ? [scopedPartition] : [],
    ...(savedScope ? { filesystem_id: savedScope.filesystem_id } : {}),
    options: {
      ...options,
      ...filter.rule,
      scope: savedScope?.entry
        ? {
            partition_id: savedScope.partition_id,
            entry: savedScope.entry,
            recursive: true,
          }
        : validContext && scope === "@folder"
          ? {
              partition_id: validContext.partition,
              entry: validContext.entry,
              recursive: true,
            }
          : null,
    },
  };
  const queryKey = JSON.stringify(query);
  const observedIndexes = useRef(indexes);
  useEffect(() => {
    const previous = observedIndexes.current;
    observedIndexes.current = indexes;
    const partitions = scopedPartition
      ? [scopedPartition]
      : new Set([...Object.keys(previous), ...Object.keys(indexes)]);
    for (const id of partitions) {
      if (
        (previous[id]?.index_version ?? null) !==
        (indexes[id]?.index_version ?? null)
      ) {
        setRefreshWanted((n) => n + 1);
        break;
      }
    }
  }, [indexes, scopedPartition]);

  function invalidate(keepCurrent = false) {
    ++sequence.current;
    const ids = [
      pendingId.current,
      keepCurrent ? null : queryId.current,
    ].filter((id) => id !== null);
    pendingId.current = null;
    if (!keepCurrent) queryId.current = null;
    return ids;
  }

  function release(ids: number[]) {
    return Promise.all(
      ids.map((id) => indexRequest({ search_cancel: { id } })),
    );
  }

  async function load(token: number, background: boolean) {
    const replaces = background ? null : replacedId.current;
    replacedId.current = null;
    const start = await indexRequest({
      search_start: {
        ...JSON.parse(queryKey),
        ...(replaces === null ? {} : { replaces }),
      },
    });
    if (start.kind !== "search_started") throw Error("搜索响应无效");
    const id = start.data;
    if (token !== sequence.current) {
      void indexRequest({ search_cancel: { id } }).catch(() => {});
      return;
    }
    pendingId.current = id;
    let page: SearchPage;
    do {
      page = await readBatch(id, 0, background ? 1 : 32, true);
      if (token !== sequence.current) return;
    } while (!page.done && token === sequence.current);
    if (token !== sequence.current) return;

    let nextCache = new Map(page.rows.map((row, index) => [index, row]));
    if (background) {
      while (token === sequence.current) {
        const position = viewportPosition.current;
        const wait = refreshDelay - (Date.now() - position.lastScroll);
        if (wait > 0) {
          await pause(wait);
          continue;
        }
        const { start, end, logicalTop } = virtualWindow(
          page.total,
          position.logicalTop,
          position.height,
          position.layout,
        );
        const rows = new Map<number, SearchHit>();
        let offset = start;
        while (offset < end) {
          const batch = await readBatch(
            id,
            offset,
            Math.min(100, end - offset),
          );
          if (token !== sequence.current) return;
          if (position !== viewportPosition.current) break;
          batch.rows.forEach((row, index) => rows.set(offset + index, row));
          if (!batch.done || !batch.rows.length)
            throw Error("搜索结果分页不完整，请重试");
          if (offset + batch.rows.length >= end) {
            offset = end;
          } else {
            if (batch.next !== offset + batch.rows.length)
              throw Error("搜索结果分页不完整，请重试");
            offset = batch.next;
          }
        }
        if (position !== viewportPosition.current) continue;
        nextCache = rows;
        setRestore({ logicalTop });
        anchor.current = cursor.current =
          Math.floor(logicalTop / position.layout.rowHeight) *
          position.layout.columns;
        break;
      }
    } else {
      anchor.current = cursor.current = 0;
      setResultVersion((n) => n + 1);
    }
    if (token !== sequence.current) return;
    pendingId.current = null;
    const previous = queryId.current;
    queryId.current = id;
    if (previous !== null && previous !== id)
      void release([previous]).catch(() => {});
    setTotal(page.total);
    setGroups(page.groups);
    setMissing(page.missing);
    setUnavailable(page.unavailable);
    setCache(nextCache);
    setSelection([]);
    selectedSnapshot.current = null;
    setMenu(null);
    setError("");
    setBusy(false);
    setRefreshing(false);
  }

  function fail(token: number, e: unknown, background: boolean) {
    if (token !== sequence.current) return;
    const ids = invalidate(background);
    void release(ids).catch(() => {});
    const reason = String(e).replace(/^Error: /, "");
    const message =
      savedScope && reason.includes("搜索目录已不存在")
        ? "书签的目录已不存在，请重新选择分区范围。"
        : reason;
    if (background) setRefreshError(message);
    else {
      setTotal(0);
      setCache(new Map());
      setError(message);
    }
    setBusy(false);
    setRefreshing(false);
  }

  useEffect(() => {
    if (composing) return;
    const typed = previousText.current !== text;
    previousText.current = text;
    const token = ++sequence.current;
    handledRefresh.current = refreshWanted;
    setBusy(true);
    setRefreshing(false);
    setError("");
    setRefreshError("");
    setRestore(null);
    setSelection([]);
    selectedSnapshot.current = null;
    setMenu(null);
    setEditing(null);
    setUnavailable([]);
    setGroups(0);
    setMissing(0);
    if (invalidScope) {
      setTotal(0);
      setCache(new Map());
      setError("书签的搜索范围已失效，请重新选择分区范围。");
      setBusy(false);
      return;
    }
    const timer = window.setTimeout(submit, typed ? inputDelay : 0);
    function submit() {
      window.clearTimeout(timer);
      submitPending.current = null;
      if (token === sequence.current)
        void load(token, false).catch((e) => fail(token, e, false));
    }
    submitPending.current = submit;
    return () => {
      window.clearTimeout(timer);
      submitPending.current = null;
      const ids = invalidate();
      replacedId.current = ids[0] ?? replacedId.current;
      void release(ids).catch(() => {});
    };
  }, [queryKey, revision, partitionKey, composing, invalidScope]);

  useEffect(() => {
    if (
      !active ||
      busy ||
      invalidScope ||
      exportSnapshot !== null ||
      refreshing ||
      editing !== null ||
      refreshWanted === handledRefresh.current
    )
      return;
    let timer: number;
    function startWhenStill() {
      const wait =
        refreshDelay - (Date.now() - viewportPosition.current.lastScroll);
      if (wait > 0) {
        timer = window.setTimeout(startWhenStill, wait);
        return;
      }
      handledRefresh.current = refreshWanted;
      const token = ++sequence.current;
      setMenu(null);
      setRefreshError("");
      setRefreshing(true);
      void load(token, true).catch((e) => fail(token, e, true));
    }
    timer = window.setTimeout(startWhenStill, refreshDelay);
    return () => window.clearTimeout(timer);
  }, [
    active,
    busy,
    invalidScope,
    exportSnapshot,
    refreshing,
    editing,
    refreshWanted,
    queryKey,
    revision,
  ]);

  useEffect(() => {
    const id = queryId.current;
    const token = sequence.current;
    if (id === null || !active || busy || refreshing || !total) return;
    let start = range[0];
    const end = Math.min(total, range[1]);
    while (start < end && cache.has(start)) start++;
    if (start >= end) return;
    let live = true;
    void readBatch(id, start, Math.min(100, end - start))
      .then((page) => {
        if (!live || token !== sequence.current) return;
        if (!page.rows.length) throw Error("搜索结果页为空，请重新搜索");
        setCache((previous) => {
          const next = new Map(
            [...previous].filter(([i]) => i >= range[0] - 100 && i < end + 100),
          );
          page.rows.forEach((row, i) => next.set(start + i, row));
          return next;
        });
      })
      .catch((e) => {
        if (live && token === sequence.current) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [cache, range, total, busy, refreshing, active, resultVersion]);
  const selected = (i: number) => inSpans(selection, i);
  const count = useMemo(() => {
    let total = 0;
    for (let i = 0; i < selection.length; i += 2)
      total += selection[i + 1] - selection[i];
    return total;
  }, [selection]);
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
      setSelection([
        Math.min(anchor.current, i),
        Math.max(anchor.current, i) + 1,
      ]);
    } else if (ctrl) {
      setSelection((s) => toggleSpans(s, [i, i + 1]));
      anchor.current = i;
    } else {
      anchor.current = i;
      setSelection([i, i + 1]);
    }
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
    if (
      queryId.current === null ||
      (override === undefined && count > maxFileSelection)
    )
      return null;
    const positions: number[] = [];
    let range: [number, number] | null = null;
    if (override !== undefined) range = [override, override];
    else if (selection.length === 2) range = [selection[0], selection[1] - 1];
    else {
      for (let s = 0; s < selection.length; s += 2)
        for (let i = selection[s]; i < selection[s + 1]; i++) positions.push(i);
    }
    return {
      origin: { kind: "search", id: queryId.current, range },
      entries: [],
      positions,
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
    if (queryId.current === null) return;
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
  const facts = {
    count,
    online,
    sameParent,
    singleFile: count === 1 && selectedRow?.file.kind === "file",
    folder: count === 1 && selectedRow?.file.kind === "folder",
    blocked:
      busy ||
      composing ||
      refreshing ||
      editing !== null ||
      queryId.current === null,
  };
  const selectAll = () => setSelection(total ? [0, total] : []);
  const changeSort = (sort: string, descending: boolean) =>
    setOptions((current) => ({ ...current, sort, descending }));
  const presetEntries = useMemo(
    () =>
      filterPresetEntries(categories, presets).map((entry) => ({
        ...entry,
        selection: selectFilter(entry.id, categories, presets),
      })),
    [categories, presets],
  );
  const filterKey = JSON.stringify(filter.rule);
  const selectedPreset = presetEntries.find(
    ({ id, selection }) =>
      JSON.stringify(selection.rule) === filterKey &&
      (id === null || !filter.id || id === filter.id),
  );
  const dispatch = useFileCommands(
    facts,
    menuAction,
    active,
    onCommands,
    [
      ...fileViewOptions(view, setView),
      {
        ...workspaceCommand("preview", () => setPreview((v) => !v)),
        checked: preview,
      },
    ],
    !busy && !refreshing && editing === null
      ? () => setRevision((n) => n + 1)
      : undefined,
    onCommands && {
      ...(onBookmark && {
        bookmark: [
          workspaceCommand(
            "bookmark",
            () => {
              const asset = assets?.find(
                (item) => item.registration.partition_id === scopedPartition,
              );
              if (scopedPartition && !asset) return;
              const { scope, ...conditions } = query.options;
              onBookmark({
                text,
                names_only: namesOnly,
                options: conditions,
                scope: asset
                  ? {
                      partition_id: asset.registration.partition_id,
                      filesystem_id: asset.registration.filesystem_id,
                      entry: scope?.entry ?? null,
                    }
                  : null,
              });
            },
            invalidScope ||
              (!!partition && !scope) ||
              (!!scopedPartition &&
                !assets?.some(
                  (item) => item.registration.partition_id === scopedPartition,
                )) ||
              composing ||
              editing !== null,
          ),
        ],
      }),
      export: [
        workspaceCommand(
          "export",
          () => {
            const id = queryId.current;
            if (id === null) return;
            const spans: [number, number][] = [];
            for (let i = 0; i < selection.length; i += 2)
              spans.push([selection[i], selection[i + 1]]);
            setExportSnapshot({ id, total, spans, count });
          },
          facts.blocked || !!error || !total,
        ),
      ],
      selection: [
        workspaceCommand("selectAll", selectAll, facts.blocked || !total),
        {
          label: "反选",
          disabled: facts.blocked || !total,
          run: () =>
            setSelection((current) => toggleSpans(current, [0, total])),
        },
      ],
      ...sortCommands(
        searchColumns,
        options.sort,
        options.descending,
        changeSort,
      ),
      search: [
        {
          ...workspaceCommand(
            "caseSensitive",
            () =>
              setOptions((o) => ({ ...o, case_sensitive: !o.case_sensitive })),
            composing || editing !== null,
          ),
          checked: options.case_sensitive,
        },
        {
          ...workspaceCommand(
            "wholeWord",
            () =>
              setOptions((o) => ({
                ...o,
                mode: o.mode === "whole_word" ? "contains" : "whole_word",
              })),
            composing || editing !== null,
          ),
          checked: options.mode === "whole_word",
        },
        {
          ...workspaceCommand(
            "path",
            () => setNamesOnly((only) => !only),
            composing || editing !== null,
          ),
          checked: !namesOnly,
        },
        {
          ...workspaceCommand(
            "regex",
            () =>
              setOptions((o) => ({
                ...o,
                mode: o.mode === "regex" ? "contains" : "regex",
              })),
            composing || editing !== null,
          ),
          checked: options.mode === "regex",
        },
        {
          label: "仅在线",
          checked: options.online_only,
          run: () => setOptions((o) => ({ ...o, online_only: !o.online_only })),
        },
        {
          label: "包含历史记录",
          checked: options.include_history,
          run: () =>
            setOptions((o) => ({ ...o, include_history: !o.include_history })),
        },
        { label: "高级搜索…", run: () => setFilterOpen("search") },
      ],
      presets: [
        ...presetEntries.map(({ id, name, selection }) => ({
          label: name,
          checked: selectedPreset?.id === id,
          radio: true,
          run: () => setFilter(selection),
        })),
        ...(!selectedPreset
          ? [
              {
                label: "自定义",
                checked: true,
                radio: true,
                disabled: true,
                run: () => {},
              },
            ]
          : []),
        ...(onSavePresets
          ? [
              {
                label: "将当前筛选另存为预设…",
                disabled: !hasFileFilter(filter.rule) || savingFilters,
                run: () => setFilterOpen("save"),
              },
            ]
          : []),
      ],
    },
  );
  return (
    <div className="assets-app search-app">
      {exportSnapshot && (
        <SearchExport
          snapshot={exportSnapshot}
          onClose={() => setExportSnapshot(null)}
        />
      )}
      {onBack && <button onClick={onBack}>硬盘管理</button>}
      <div className="search-toolbar">
        <WorkspaceIcon name="search" className="search-field-icon" />
        <input
          ref={input}
          type="search"
          aria-label="搜索文件或路径"
          placeholder="搜索文件名或路径…"
          value={text}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={() => setComposing(false)}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !composing &&
              !e.nativeEvent.isComposing &&
              e.keyCode !== 229
            ) {
              submitPending.current?.();
            }
          }}
          onChange={(e) => {
            setText(e.target.value);
            onTextChange?.(e.target.value);
          }}
        />
        <select
          aria-label="分区范围"
          value={scope}
          onChange={(e) => setPartition(e.target.value)}
        >
          <option value="">全部已登记分区</option>
          {savedScope && (
            <option value="@bookmark" disabled={invalidScope}>
              {invalidScope
                ? "书签范围已失效"
                : `书签范围：${scopedAsset?.registration.name}${savedScope.entry ? "（目录及子目录）" : ""}`}
            </option>
          )}
          {validContext && (
            <>
              <option value="@partition">原浏览分区</option>
              <option value="@folder">{validContext.name}及子目录</option>
            </>
          )}
          {partitions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          aria-label="筛选器"
          aria-haspopup="dialog"
          aria-describedby={filterActive ? filterStatusId : undefined}
          className="search-filter-button"
          onClick={() => setFilterOpen("search")}
        >
          <WorkspaceIcon name="filter" />
          筛选器
          {filterActive && <span id={filterStatusId}>已启用</span>}
        </button>
        {active && filterOpen && (
          <SearchFilters
            mode={filterOpen}
            initialOptions={options}
            categories={categories}
            initialFilter={filter}
            initialNamesOnly={namesOnly}
            defaultNamesOnly={!includePath}
            presets={presets}
            onSavePresets={onSavePresets}
            savingPresets={savingFilters}
            onClose={() => setFilterOpen(false)}
            onApply={(nextFilter, nextOptions, nextNamesOnly) => {
              setFilter(nextFilter);
              setOptions(nextOptions);
              setNamesOnly(nextNamesOnly);
              setFilterOpen(false);
            }}
          />
        )}
      </div>
      {error && (
        <p role="alert" className="asset-error">
          {error}
        </p>
      )}
      {refreshError && (
        <p role="alert" className="asset-error">
          后台刷新失败：{refreshError}{" "}
          <button
            onClick={() => setRefreshWanted((n) => n + 1)}
            disabled={refreshing}
          >
            重试
          </button>
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
      <div
        className="search-body"
        inert={total > 0 && (busy || composing)}
        onClick={() => setMenu(null)}
      >
        <VirtualResults
          overlay={
            !busy &&
            !error &&
            !total && (
              <p className="search-empty-overlay" role="status">
                <WorkspaceIcon name="search" />
                没有匹配的文件
              </p>
            )
          }
          grid={grid}
          active={active}
          sort={options.sort}
          descending={options.descending}
          onSort={(sort) =>
            changeSort(sort, options.sort === sort && !options.descending)
          }
          total={total}
          revision={resultVersion}
          onRange={viewport}
          onPosition={onPosition}
          restore={restore}
          marquee={{
            enabled: active && !facts.blocked,
            version: queryId.current,
            commit(area, toggle) {
              const spans = rectangleSpans(area, total);
              if (spans.length) anchor.current = cursor.current = spans[0];
              setSelection((previous) =>
                toggle ? toggleSpans(previous, spans) : spans,
              );
            },
          }}
          onKeyDown={(e, { columns, reveal, scrollTo }) => {
            if (fileEditingKey(e)) return;
            if (
              fileKey(e, dispatch, () => {
                if (count) {
                  const rect = e.currentTarget.getBoundingClientRect();
                  setMenu({
                    x: rect.left + 20,
                    y: rect.top + 45,
                  });
                }
              })
            )
              return;
            if (
              !e.metaKey &&
              !e.repeat &&
              eventShortcut(e.nativeEvent) ===
                workspaceShortcuts.selectAll[1] &&
              !facts.blocked
            ) {
              e.preventDefault();
              selectAll();
            }
            if (e.key === "Escape") setMenu(null);
            if (grid && (e.key === "Home" || e.key === "End") && total) {
              e.preventDefault();
              e.currentTarget.focus({ preventScroll: true });
              scrollTo(e.key === "Home" ? 0 : Infinity);
            }
            const step =
              e.key === "ArrowDown"
                ? columns
                : e.key === "ArrowUp"
                  ? -columns
                  : grid && e.key === "ArrowRight"
                    ? 1
                    : grid && e.key === "ArrowLeft"
                      ? -1
                      : 0;
            if (step && total) {
              e.preventDefault();
              const i = Math.max(0, Math.min(total - 1, cursor.current + step));
              choose(i, e.shiftKey, false);
              reveal(i);
              if (grid) {
                const list = e.currentTarget;
                requestAnimationFrame(() =>
                  list
                    .querySelector<HTMLElement>(`[data-index="${i}"]`)
                    ?.focus({ preventScroll: true }),
                );
              }
            }
            if (e.key === "Enter" && selectedRow) open(selectedRow);
          }}
          row={(i, _scrollToRow, _columns, frame, detailColumns) => {
            const marked = frame
              ? frame.toggle
                ? selected(i) !== frame.hit
                : frame.hit
              : selected(i);
            const cached = cache.get(i);
            const r = cached ? currentHit(cached) : undefined;
            const path = r
              ? (r.connection === "online" ? r.mounts[0] || "" : "") +
                r.file.path
              : "";
            const location = r ? `${r.partition_name} · ${path}` : "";
            return (
              <FileListItem
                key={i}
                tabIndex={-1}
                view={view}
                index={i}
                total={total}
                selected={marked}
                file={r?.file}
                active={active}
                connection={r?.connection}
                reference={
                  r && {
                    partition_id: r.partition_id,
                    filesystem_id: r.filesystem_id,
                    generation: String(r.generation),
                    revision: String(r.revision),
                    entry: r.file.entry,
                  }
                }
                editing={
                  editing === i
                    ? {
                        onCancel: () => setEditing(null),
                        onSubmit: (name) => {
                          setEditing(null);
                          perform({ rename: name }, i);
                        },
                      }
                    : undefined
                }
                location={location}
                badge={r?.hard_link ? "硬链接" : undefined}
                onClick={(e) => {
                  if (suppressClick.current) {
                    suppressClick.current = false;
                    return;
                  }
                  if (grid) e.currentTarget.focus({ preventScroll: true });
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
                  e.currentTarget.focus();
                  if (!selected(i)) choose(i, false, false);
                  setMenu({
                    x: Math.min(e.clientX, window.innerWidth - 270),
                    y: Math.min(e.clientY, window.innerHeight - 60),
                  });
                }}
              >
                {!grid &&
                  (!r ? (
                    <td colSpan={detailColumns.length}>正在读取…</td>
                  ) : (
                    detailColumns.slice(1).map(({ id }) => {
                      if (id === "status") {
                        const state =
                          connectionLabel[r.connection] || "状态未确认";
                        const note = r.partial
                          ? "未完成记录"
                          : r.needs_validation
                            ? "索引待校验"
                            : "";
                        return (
                          <td
                            key={id}
                            title={note ? `${state} · ${note}` : state}
                          >
                            <span>{state}</span>
                            {note && (
                              <>
                                {" "}
                                · <span>{note}</span>
                              </>
                            )}
                          </td>
                        );
                      }
                      const text =
                        id === "path"
                          ? path
                          : id === "partition"
                            ? r.partition_name
                            : id === "group"
                              ? String(r.duplicate_group || "—")
                              : fileColumnText(r.file, id);
                      return (
                        <td
                          key={id}
                          className={id === "size" ? "result-size" : undefined}
                          title={
                            id === "size" && r.file.size !== null
                              ? `${BigInt(r.file.size).toLocaleString()} B`
                              : text
                          }
                        >
                          {text}
                        </td>
                      );
                    })
                  ))}
              </FileListItem>
            );
          }}
        />
        {preview && (
          <FileDetails
            active={active}
            selectionCount={count}
            reference={
              selectedRow
                ? {
                    partition_id: selectedRow.partition_id,
                    filesystem_id: selectedRow.filesystem_id,
                    generation: String(selectedRow.generation),
                    revision: String(selectedRow.revision),
                    entry: selectedRow.file.entry,
                  }
                : undefined
            }
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
      <FileStatusBar
        essential={
          busy || error ? (
            <span
              className={`search-summary ${busy ? "search-pending" : ""}`}
              role="status"
            >
              {busy ? "正在搜索…" : "搜索失败"}
            </span>
          ) : count > maxFileSelection ? (
            <span role="status">{fileSelectionLimit}</span>
          ) : null
        }
      >
        {!busy && !error && (
          <span
            className="search-summary"
            role="status"
          >{`${total.toLocaleString()} 条结果`}</span>
        )}
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
      </FileStatusBar>
      {active && menu && (
        <FileContextMenu
          x={menu.x}
          y={menu.y}
          count={count}
          online={facts.online}
          sameParent={facts.sameParent}
          singleFile={facts.singleFile}
          folder={facts.folder}
          blocked={facts.blocked}
          onAction={dispatch}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
