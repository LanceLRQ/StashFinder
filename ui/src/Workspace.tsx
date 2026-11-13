import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { SettingsPage } from "./Settings";
import {
  eventShortcut,
  workspaceCommand,
  workspaceShortcuts,
} from "./workspace-shortcuts";
import type { Asset } from "./asset-api";
import type { PartitionIndexes } from "./task-api";
import type { SearchHit } from "./index-api";
import BrowsePane from "./BrowsePane";
import { FileStatusBar } from "./FileOperations";
import Modal from "./Modal";
import type { FileView } from "./file-view";
import Search from "./Search";
import WindowChrome from "./WindowChrome";
import WorkspaceIcon from "./WorkspaceIcon";
import WorkspaceTabs from "./WorkspaceTabs";
import "./workspace.css";
import {
  defaultCategories,
  filterPresetEntries,
  hasFileFilter,
  noPresets,
  selectFilter,
  type Category,
  type FilterPreset,
  type FilterSelection,
  type SearchContext,
} from "./search-options";
import type { DirectoryPage } from "./browse-api";
import AppMenu, { type MenuCommand, type MenuGroup } from "./AppMenu";
import {
  editCommands,
  editableTarget,
  fileCommands,
  fileKey,
  type FileCommandContext,
} from "./file-operations";

type BrowseTab = {
  kind: "browse";
  id: number;
  title: string;
  partition: string | null;
  filesystem: string | null;
  parent: number[] | null;
  focusEntry?: number[];
  view?: FileView;
};
type SearchTab = {
  kind: "search";
  id: number;
  title: string;
  initialText: string;
  context?: SearchContext;
  origin: number;
  initialFilter?: FilterSelection;
};
const searchTitle = (text: string) =>
  text.trim() ? `搜索：${text.trim()}` : "搜索";
const pages = {
  overview: { id: -1, title: "概览", kind: "overview" },
  development: { id: -4, title: "开发验证", kind: "development" },
} as const;
type AuxiliaryPage = keyof typeof pages;
type Tab = BrowseTab | SearchTab | (typeof pages)[AuxiliaryPage];
const newTab = (id: number): BrowseTab => ({
  kind: "browse",
  id,
  title: "浏览",
  partition: null,
  filesystem: null,
  parent: null,
});
export default function Workspace({
  assets,
  includePath,
  request,
  browseRequest,
  categories = defaultCategories,
  browseFilter = null,
  filterPresets = noPresets,
  onBrowseFilter,
  onSavePresets,
  savingFilters,
  indexes,
  automatic,
  paused = [],
  visible = true,
  searchReady = true,
  startupReady,
  device,
  panels,
  notice,
  onSettings,
  onExit,
}: {
  assets: Asset[];
  includePath: boolean;
  request: { text: string; revision: number };
  browseRequest?: { partition: string; revision: number };
  categories?: Category[];
  browseFilter?: string | null;
  filterPresets?: FilterPreset[];
  onBrowseFilter?: (id: string | null) => void;
  onSavePresets?: (presets: FilterPreset[]) => Promise<boolean>;
  savingFilters?: boolean;
  indexes?: PartitionIndexes;
  automatic?: boolean;
  paused?: string[];
  visible?: boolean;
  searchReady?: boolean;
  startupReady?: boolean;
  device?: {
    register: () => void;
    refresh: () => void;
    retry: () => void;
    busy: boolean;
    working: boolean;
    error: string;
  };
  panels?: Partial<Record<AuxiliaryPage, ReactNode>>;
  notice?: ReactNode;
  onSettings?: (page: SettingsPage) => void;
  onExit?: () => void;
}) {
  const [tabs, setTabs] = useState<Tab[]>(() =>
    panels?.overview ? [pages.overview] : [newTab(1)],
  );
  const [active, setActive] = useState<number>(tabs[0].id);
  const sequence = useRef(1);
  const root = useRef<HTMLDivElement>(null);
  const [partitionPickerOpen, setPartitionPickerOpen] = useState(false);
  const locations = useRef(new Map<number, DirectoryPage>());
  const searchFocus = useRef(false);
  const [commands, setCommands] = useState<FileCommandContext | null>(null);
  const currentTab = tabs.find((tab) => tab.id === active);
  const browseSelection = useMemo(
    () => selectFilter(browseFilter, categories, filterPresets),
    [browseFilter, categories, filterPresets],
  );
  const hasFileStatusBar =
    currentTab?.kind === "overview" ||
    (currentTab?.kind === "search" && searchReady) ||
    (currentTab?.kind === "browse" &&
      (!currentTab.partition ||
        assets.some(
          ({ registration }) =>
            registration.partition_id === currentTab.partition &&
            registration.filesystem_id === currentTab.filesystem,
        )));
  const refreshCurrent =
    currentTab?.kind === "overview"
      ? device && !device.busy
        ? device.refresh
        : undefined
      : currentTab?.kind === "browse" || currentTab?.kind === "search"
        ? commands?.refresh
        : undefined;
  const location = useCallback((id: number, page: DirectoryPage) => {
    locations.current.set(id, page);
  }, []);
  const activeRef = useRef(active);
  activeRef.current = active;
  const search = useCallback(
    (initialText: string, context?: SearchContext) => {
      const id = ++sequence.current;
      const tab: SearchTab = {
        id,
        kind: "search",
        initialText,
        context,
        title: searchTitle(initialText),
        origin: activeRef.current,
        initialFilter: searchReady ? browseSelection : undefined,
      };
      setTabs((previous) => [...previous, tab]);
      setActive(id);
    },
    [browseSelection, searchReady],
  );
  const searchRevision = useRef(0);
  useEffect(() => {
    if (request.revision > searchRevision.current) {
      searchRevision.current = request.revision;
      search(request.text);
    }
  }, [request, search]);
  const browseRevision = useRef(0);
  useEffect(() => {
    if (!browseRequest || browseRevision.current === browseRequest.revision)
      return;
    const asset = assets.find(
      (a) => a.registration.partition_id === browseRequest.partition,
    );
    if (!asset) return;
    browseRevision.current = browseRequest.revision;
    const existing = tabs.find((t) => {
      if (
        t.kind !== "browse" ||
        t.partition !== browseRequest.partition ||
        t.filesystem !== asset.registration.filesystem_id
      )
        return false;
      const location = locations.current.get(t.id);
      return location ? location.ancestors.length === 1 : t.parent === null;
    });
    if (existing) {
      setActive(existing.id);
      return;
    }
    const empty = tabs.find(
      (t) =>
        t.kind === "browse" &&
        !t.partition &&
        (t.id === activeRef.current || activeRef.current === pages.overview.id),
    );
    const id = empty?.id ?? ++sequence.current;
    const tab: BrowseTab = {
      kind: "browse",
      id,
      title: asset.registration.name,
      partition: asset.registration.partition_id,
      filesystem: asset.registration.filesystem_id,
      parent: null,
    };
    setTabs((previous) =>
      empty ? previous.map((t) => (t.id === id ? tab : t)) : [...previous, tab],
    );
    setActive(id);
  }, [browseRequest, assets, tabs]);
  useEffect(() => {
    if (!panels?.overview) return;
    const removed = tabs.filter(
      (tab) =>
        tab.kind === "browse" &&
        tab.partition &&
        !assets.some(
          (asset) => asset.registration.partition_id === tab.partition,
        ),
    );
    if (!removed.length) return;
    for (const tab of removed) locations.current.delete(tab.id);
    setTabs(tabs.filter((tab) => !removed.includes(tab)));
    if (removed.some((tab) => tab.id === active)) setActive(pages.overview.id);
  }, [assets, tabs, active, panels?.overview]);
  const title = useCallback(
    (id: number, value: string) =>
      setTabs((previous) =>
        previous.some((t) => t.id === id && t.title !== value)
          ? previous.map((t) =>
              (t.kind === "browse" || t.kind === "search") && t.id === id
                ? { ...t, title: value }
                : t,
            )
          : previous,
      ),
    [],
  );
  function add() {
    const id = ++sequence.current;
    setTabs((previous) => [...previous, newTab(id)]);
    setActive(id);
  }
  function close(id: number) {
    if (id === pages.overview.id) return;
    locations.current.delete(id);
    const closed = tabs.find((t) => t.id === id);
    const remaining = tabs.filter((t) => t.id !== id);
    if (!remaining.length) {
      const next = ++sequence.current;
      setTabs([newTab(next)]);
      setActive(next);
    } else {
      setTabs(remaining);
      if (active === id)
        setActive(
          closed?.kind === "search" &&
            remaining.some((t) => t.id === closed.origin)
            ? closed.origin
            : remaining[
                Math.min(
                  tabs.findIndex((t) => t.id === id),
                  remaining.length - 1,
                )
              ].id,
        );
    }
  }
  function openPage(page: AuxiliaryPage) {
    const tab = pages[page];
    setTabs((previous) =>
      previous.some((item) => item.id === tab.id)
        ? previous
        : [...previous, tab],
    );
    setActive(tab.id);
  }
  function choosePartition(id: number, partition: string) {
    const chosen = assets.find(
      (a) => a.registration.partition_id === partition,
    );
    if (!chosen) return;
    setTabs((previous) =>
      previous.map((tab) =>
        tab.kind === "browse" && tab.id === id
          ? {
              ...tab,
              partition,
              filesystem: chosen.registration.filesystem_id,
              parent: null,
              focusEntry: undefined,
              title: chosen.registration.name,
            }
          : tab,
      ),
    );
  }
  function locate(
    hit: Pick<
      SearchHit,
      "partition_id" | "filesystem_id" | "partition_name" | "file"
    >,
    folder = false,
  ) {
    const id = ++sequence.current;
    setTabs((previous) => [
      ...previous,
      {
        kind: "browse",
        id,
        title: hit.partition_name,
        partition: hit.partition_id,
        filesystem: hit.filesystem_id,
        parent: folder ? hit.file.entry : hit.file.parent,
        focusEntry: folder ? undefined : hit.file.entry,
      },
    ]);
    setActive(id);
  }
  function focusSearch() {
    const input = root.current?.querySelector<HTMLInputElement>(
      'section:not([hidden]) input[type="search"]',
    );
    if (input) {
      input.focus();
      input.select();
    } else {
      searchFocus.current = true;
      const file = [...tabs]
        .reverse()
        .find((tab) => tab.kind === "browse" || tab.kind === "search");
      if (file) setActive(file.id);
      else add();
    }
  }
  function cycleTab(direction: number) {
    const ids = tabs.map((tab) => tab.id);
    setActive(ids[(ids.indexOf(active) + direction + ids.length) % ids.length]);
  }
  const mainCommands = {
    browse: workspaceCommand("browse", add),
    search: workspaceCommand("search", () => search("")),
    close: workspaceCommand(
      "close",
      () => close(active),
      active === pages.overview.id,
    ),
    focus: workspaceCommand("focus", focusSearch),
    refresh: workspaceCommand(
      "refresh",
      () => refreshCurrent?.(),
      !refreshCurrent,
    ),
    settings: workspaceCommand(
      "settings",
      () => onSettings?.("general"),
      !onSettings,
    ),
    exit: workspaceCommand("exit", () => onExit?.(), !onExit),
    nextTab: workspaceCommand("nextTab", () => cycleTab(1)),
    previousTab: workspaceCommand("previousTab", () => cycleTab(-1)),
  };
  const actions = commands?.actions;
  const shortcuts: MenuCommand[] = [
    ...Object.values(mainCommands),
    ...(commands?.views ?? []),
    ...(actions?.selection ?? []),
    ...(actions?.navigation ?? []),
    ...(actions?.search ?? []),
  ];
  useEffect(() => {
    function keyDown(e: KeyboardEvent) {
      if (
        !root.current ||
        root.current.closest("[hidden]") ||
        e.metaKey ||
        e.isComposing ||
        e.keyCode === 229 ||
        e.key === "Process" ||
        e.defaultPrevented ||
        document.querySelector(
          'dialog[open], [role="dialog"], [role="alertdialog"]',
        ) ||
        (e.target instanceof Element && e.target.closest('[role="menu"]'))
      )
        return;

      const shortcut = eventShortcut(e);
      const command = shortcuts.find((item) => item.shortcut === shortcut);
      if (command) {
        const editing = editableTarget(e.target);
        const searchInput =
          e.target instanceof HTMLInputElement &&
          e.target.type === "search" &&
          currentTab?.kind === "search";
        if (
          editing &&
          (shortcut === workspaceShortcuts.selectAll[1] ||
            shortcut === workspaceShortcuts.up[1] ||
            (actions?.search?.includes(command) && !searchInput))
        )
          return;
        e.preventDefault();
        if (!e.repeat && !command.disabled) command.run();
        return;
      }
      if (commands) fileKey(e, commands.dispatch);
    }
    document.addEventListener("keydown", keyDown);
    return () => document.removeEventListener("keydown", keyDown);
  });
  useEffect(() => {
    root.current
      ?.querySelector('[role="tab"][aria-selected="true"]')
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    if (searchFocus.current) {
      searchFocus.current = false;
      root.current
        ?.querySelector<HTMLInputElement>(
          'section:not([hidden]) input[type="search"]',
        )
        ?.focus();
    }
  }, [active]);
  const layouts = (commands?.views ?? []).filter((item) => item.radio);
  const presets =
    currentTab?.kind === "browse" && onBrowseFilter
      ? filterPresetEntries(categories, filterPresets).map(({ id, name }) => ({
          label: name,
          radio: true,
          checked: id === browseFilter,
          disabled: savingFilters,
          run: () => onBrowseFilter(id),
        }))
      : (actions?.presets ?? []);
  const groups: MenuGroup[] = [
    {
      label: "文件",
      accessKey: "f",
      items: [
        mainCommands.browse,
        mainCommands.search,
        mainCommands.close,
        ...(currentTab?.kind === "browse"
          ? [
              {
                label: "切换浏览分区…",
                disabled: !assets.length,
                run: () => setPartitionPickerOpen(true),
              },
            ]
          : []),
        ...(device
          ? [
              {
                label: "登记分区…",
                run: device.register,
                disabled: startupReady !== true || device.busy,
              },
            ]
          : []),
        ...(onExit ? [mainCommands.exit] : []),
      ],
    },
    {
      label: "编辑",
      accessKey: "e",
      items: [
        ...editCommands.slice(0, 5).map(editCommand),
        actions?.selection?.[0] ??
          workspaceCommand("selectAll", () => {}, true),
        ...editCommands.slice(5).map(editCommand),
      ],
    },
    {
      label: "查看",
      accessKey: "v",
      items: [
        mainCommands.refresh,
        ...(commands?.views.filter((item) => !item.radio) ?? []),
        ...(layouts.length ? [{ label: "布局", items: layouts }] : []),
        ...(actions?.sorting?.length
          ? [{ label: "排序", items: actions.sorting }]
          : []),
        ...(actions?.navigation?.length
          ? [{ label: "前往", items: actions.navigation }]
          : []),
      ],
    },
    {
      label: "搜索",
      accessKey: "s",
      items: [
        mainCommands.focus,
        ...(actions?.search ?? []),
        ...(presets.length
          ? [{ label: "筛选预设", items: presets.filter((item) => item.radio) }]
          : []),
        ...presets.filter((item) => !item.radio),
        ...(onSettings
          ? [{ label: "管理筛选预设…", run: () => onSettings("presets") }]
          : []),
      ],
    },
    {
      label: "工具",
      accessKey: "t",
      items: [
        ...(onSettings ? [mainCommands.settings] : []),
        ...(device
          ? [{ label: "刷新设备", run: device.refresh, disabled: device.busy }]
          : []),
        ...(panels?.development
          ? [{ label: "开发验证", run: () => openPage("development") }]
          : []),
      ],
    },
    ...(onSettings
      ? [
          {
            label: "帮助",
            accessKey: "h",
            items: [
              { label: "快捷键", run: () => onSettings("shortcuts") },
              { label: "关于与诊断", run: () => onSettings("about") },
            ],
          },
        ]
      : []),
  ];
  function editCommand(id: (typeof editCommands)[number]): MenuCommand {
    return {
      label: fileCommands[id][0],
      shortcut: fileCommands[id][1],
      disabled: !commands?.enabled(id),
      run: () => commands?.dispatch(id),
    };
  }
  function tabButton(tab: Tab) {
    return (
      <div
        className={`workspace-tab ${active === tab.id ? "is-active" : ""}`}
        key={tab.id}
      >
        <button
          role="tab"
          aria-selected={active === tab.id}
          aria-controls={`file-tab-${tab.id}`}
          onClick={() => setActive(tab.id)}
          title={tab.title}
          aria-label={tab.kind === "overview" ? tab.title : undefined}
        >
          <WorkspaceIcon
            name={
              tab.kind === "browse"
                ? "folder"
                : tab.kind === "overview"
                  ? "home"
                  : tab.kind
            }
          />
          {tab.kind !== "overview" && <span>{tab.title}</span>}
        </button>
        {tab.kind !== "overview" && (
          <button
            className="tab-close"
            aria-label={`关闭${tab.title}标签页`}
            onClick={() => close(tab.id)}
          >
            ×
          </button>
        )}
      </div>
    );
  }
  const home = tabs.find((tab) => tab.kind === "overview");
  return (
    <div className="file-workspace" ref={root}>
      <WindowChrome>
        <WorkspaceTabs
          active={active}
          home={home && tabButton(home)}
          onAdd={add}
        >
          {tabs.filter((tab) => tab.kind !== "overview").map(tabButton)}
        </WorkspaceTabs>
      </WindowChrome>
      <AppMenu groups={groups} />
      {partitionPickerOpen && (
        <Modal
          title="切换浏览分区"
          onClose={() => setPartitionPickerOpen(false)}
        >
          <ul className="browse-partitions">
            {assets.map(({ registration }) => (
              <li key={registration.partition_id}>
                <button
                  onClick={() => {
                    setPartitionPickerOpen(false);
                    choosePartition(active, registration.partition_id);
                  }}
                >
                  {registration.name}
                </button>
              </li>
            ))}
          </ul>
        </Modal>
      )}
      {notice}
      {device?.error && active !== pages.overview.id && (
        <p className="browse-message desktop-error" role="alert">
          {assets.length ? "设备状态可能已过期：" : "无法读取设备："}
          {device.error} <button onClick={device.retry}>重试</button>
        </p>
      )}
      {device?.working && active !== pages.overview.id && (
        <p className="browse-message" role="status">
          正在处理设备信息…
        </p>
      )}
      {tabs
        .filter((tab): tab is BrowseTab => tab.kind === "browse")
        .map((tab) => {
          const asset = assets.find(
            (a) =>
              a.registration.partition_id === tab.partition &&
              a.registration.filesystem_id === tab.filesystem,
          );
          return (
            <section
              role="tabpanel"
              id={`file-tab-${tab.id}`}
              aria-label={tab.title}
              className="workspace-panel"
              hidden={active !== tab.id}
              key={tab.id}
            >
              {tab.partition && !asset ? (
                <p className="browse-message" role="alert">
                  该分区记录已不可用，请关闭此标签页后重新选择。
                </p>
              ) : (
                <BrowsePane
                  filter={
                    hasFileFilter(browseSelection.rule)
                      ? browseSelection.rule
                      : undefined
                  }
                  active={visible && active === tab.id}
                  view={tab.view ?? "details"}
                  onView={(view) =>
                    setTabs((previous) =>
                      previous.map((item) =>
                        item.kind === "browse" && item.id === tab.id
                          ? { ...item, view }
                          : item,
                      ),
                    )
                  }
                  key={`${tab.id}:${tab.partition}:${tab.filesystem}`}
                  id={tab.id}
                  asset={asset}
                  assets={assets}
                  initialParent={tab.parent}
                  focusEntry={tab.focusEntry}
                  onLocation={location}
                  onCommands={setCommands}
                  emptyState={
                    device &&
                    (startupReady && !assets.length ? (
                      <>
                        <p>尚未登记分区</p>
                        <button
                          disabled={device.busy}
                          onClick={device.register}
                        >
                          登记分区…
                        </button>
                      </>
                    ) : !device.error && !startupReady && !assets.length ? (
                      <p role="status">正在读取已登记分区…</p>
                    ) : (
                      <p>选择已登记分区以浏览文件</p>
                    ))
                  }
                  indexVersion={
                    indexes?.[tab.partition ?? ""]?.index_version ?? ""
                  }
                  automatic={
                    automatic === true && !paused.includes(tab.partition ?? "")
                  }
                  onLocate={(row) => {
                    if (asset)
                      locate({
                        partition_id: asset.registration.partition_id,
                        filesystem_id: asset.registration.filesystem_id,
                        partition_name: asset.registration.name,
                        file: row,
                      });
                  }}
                  onSearch={(text) => {
                    const page = locations.current.get(tab.id);
                    search(
                      text,
                      page && tab.partition
                        ? {
                            partition: tab.partition,
                            entry: page.location,
                            name: tab.title,
                          }
                        : undefined,
                    );
                  }}
                  onTitle={title}
                  onChoose={(partition) => choosePartition(tab.id, partition)}
                />
              )}
            </section>
          );
        })}
      {tabs
        .filter((tab): tab is SearchTab => tab.kind === "search")
        .map((tab) => (
          <section
            role="tabpanel"
            key={tab.id}
            id={`file-tab-${tab.id}`}
            aria-label={tab.title}
            className="workspace-panel"
            hidden={active !== tab.id}
          >
            {searchReady ? (
              <Search
                assets={assets}
                partitions={assets.map((a) => ({
                  id: a.registration.partition_id,
                  name: a.registration.name,
                }))}
                includePath={includePath}
                initialText={tab.initialText}
                onTextChange={(text) => title(tab.id, searchTitle(text))}
                active={visible && active === tab.id}
                onCommands={setCommands}
                onLocate={locate}
                categories={categories}
                initialFilter={tab.initialFilter ?? browseSelection}
                presets={filterPresets}
                onSavePresets={onSavePresets}
                savingFilters={savingFilters}
                indexes={indexes}
                context={tab.context}
              />
            ) : (
              <p className="browse-message" role="status">
                正在读取搜索设置…
              </p>
            )}
          </section>
        ))}
      {panels &&
        (Object.keys(panels) as AuxiliaryPage[]).map((page) => {
          const tab = pages[page];
          if (page !== "overview" && !tabs.some((item) => item.id === tab.id))
            return null;
          return (
            <section
              key={page}
              role="tabpanel"
              id={`file-tab-${tab.id}`}
              aria-label={tab.title}
              className="workspace-panel"
              hidden={active !== tab.id}
            >
              {panels[page]}
            </section>
          );
        })}
      {!hasFileStatusBar && <FileStatusBar />}
    </div>
  );
}
