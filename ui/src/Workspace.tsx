import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { Asset } from "./asset-api";
import type { SearchHit } from "./index-api";
import BrowsePane from "./BrowsePane";
import type { FileView } from "./file-view";
import Search from "./Search";
import WindowChrome from "./WindowChrome";
import WorkspaceIcon from "./WorkspaceIcon";
import "./workspace.css";
import type { Category, SearchContext } from "./search-options";
import type { DirectoryPage } from "./browse-api";
import AppMenu, { type MenuGroup } from "./AppMenu";
import {
  editCommands,
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
const pages = {
  search: { id: 0, title: "搜索", kind: "search" },
  overview: { id: -1, title: "概览", kind: "overview" },
  tasks: { id: -2, title: "任务", kind: "tasks" },
  settings: { id: -3, title: "设置", kind: "settings" },
  development: { id: -4, title: "开发验证", kind: "development" },
} as const;
type Tab = BrowseTab | (typeof pages)[keyof typeof pages];
type AuxiliaryPage = Exclude<keyof typeof pages, "search">;
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
  taskRequest = 0,
  categories,
  completedTasks = [],
  automatic,
  paused = [],
  visible = true,
  searchReady = true,
  startupReady,
  device,
  taskRefresh,
  panels,
  notice,
}: {
  assets: Asset[];
  includePath: boolean;
  request: { text: string; revision: number };
  browseRequest?: { partition: string; revision: number };
  taskRequest?: number;
  categories?: Category[];
  completedTasks?: { partition_id: string; id: string; updated: number }[];
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
  taskRefresh?: () => void;
  panels?: Partial<Record<AuxiliaryPage, ReactNode>>;
  notice?: ReactNode;
}) {
  const [tabs, setTabs] = useState<Tab[]>(() =>
    panels?.overview ? [pages.overview] : [newTab(1)],
  );
  const [active, setActive] = useState<number>(tabs[0].id);
  const [query, setQuery] = useState({ text: "", revision: 0 });
  const sequence = useRef(1);
  const root = useRef<HTMLDivElement>(null);
  const partitionPicker = useRef<HTMLDialogElement>(null);
  const returnTo = useRef(active);
  const locations = useRef(new Map<number, DirectoryPage>());
  const searchFocus = useRef(false);
  const [searchContext, setSearchContext] = useState<SearchContext>();
  const [commands, setCommands] = useState<FileCommandContext | null>(null);
  const currentTab = tabs.find((tab) => tab.id === active);
  const refreshCurrent =
    currentTab?.kind === "overview"
      ? device && !device.busy
        ? device.refresh
        : undefined
      : currentTab?.kind === "tasks"
        ? taskRefresh
        : currentTab?.kind === "browse" || currentTab?.kind === "search"
          ? commands?.refresh
          : undefined;
  const location = useCallback((id: number, page: DirectoryPage) => {
    locations.current.set(id, page);
  }, []);
  const activeRef = useRef(active);
  activeRef.current = active;
  const search = useCallback((text: string) => {
    if (activeRef.current !== pages.search.id)
      returnTo.current = activeRef.current;
    setQuery((q) => ({ text, revision: q.revision + 1 }));
    setTabs((previous) =>
      previous.some((tab) => tab.kind === "search")
        ? previous
        : [...previous, pages.search],
    );
    setActive(0);
  }, []);
  useEffect(() => {
    if (request.revision > 0) {
      setSearchContext(undefined);
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
    if (!taskRequest) return;
    setTabs((previous) =>
      previous.some((tab) => tab.id === pages.tasks.id)
        ? previous
        : [...previous, pages.tasks],
    );
    setActive(pages.tasks.id);
  }, [taskRequest]);
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
              t.kind === "browse" && t.id === id ? { ...t, title: value } : t,
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
    const remaining = tabs.filter((t) => t.id !== id);
    if (!remaining.length) {
      const next = ++sequence.current;
      setTabs([newTab(next)]);
      setActive(next);
    } else {
      setTabs(remaining);
      if (active === id)
        setActive(
          id === 0 && remaining.some((t) => t.id === returnTo.current)
            ? returnTo.current
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
  useEffect(() => {
    function keyDown(e: KeyboardEvent) {
      if (
        !root.current ||
        root.current.closest("[hidden]") ||
        e.isComposing ||
        e.keyCode === 229 ||
        e.key === "Process" ||
        e.defaultPrevented ||
        document.querySelector(
          'dialog[open], [role="dialog"], [role="alertdialog"]',
        )
      )
        return;
      if (
        e.key === "F5" &&
        !e.altKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.shiftKey
      ) {
        e.preventDefault();
        if (!e.repeat) refreshCurrent?.();
        return;
      }
      if (e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        const input = root.current.querySelector(
          'section:not([hidden]) input[type="search"]',
        ) as HTMLInputElement | null;
        if (input) {
          input.focus();
          input.select();
        } else {
          searchFocus.current = true;
          const file =
            tabs.find(
              (tab) => tab.id === returnTo.current && tab.kind === "browse",
            ) ??
            tabs.find((tab) => tab.kind === "browse" || tab.kind === "search");
          if (file) setActive(file.id);
          else add();
        }
        return;
      }
      if (e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        close(active);
        return;
      }
      if (e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        add();
        return;
      }
      if (e.ctrlKey && !e.altKey && !e.metaKey && e.key === "Tab") {
        e.preventDefault();
        const ids = tabs.map((t) => t.id);
        setActive(
          ids[
            (ids.indexOf(active) + (e.shiftKey ? ids.length - 1 : 1)) %
              ids.length
          ],
        );
        return;
      }
      if (
        commands &&
        !(e.target instanceof Element && e.target.closest('[role="menu"]'))
      ) {
        fileKey(e, commands.dispatch);
      }
    }
    document.addEventListener("keydown", keyDown);
    return () => document.removeEventListener("keydown", keyDown);
  }, [active, tabs, assets, commands, refreshCurrent]);
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
  const groups: MenuGroup[] = [
    {
      label: "文件",
      accessKey: "f",
      items: [
        { label: "新建浏览标签页", shortcut: "Ctrl+T", run: add },
        {
          label: "关闭标签页",
          shortcut: "Ctrl+W",
          disabled: active === pages.overview.id,
          run: () => close(active),
        },
        ...(currentTab?.kind === "browse"
          ? [
              {
                label: "切换浏览分区…",
                disabled: !assets.length,
                run: () => partitionPicker.current?.showModal(),
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
              { label: "刷新设备", run: device.refresh, disabled: device.busy },
            ]
          : []),
      ],
    },
    {
      label: "编辑",
      accessKey: "e",
      items: editCommands.map((id) => ({
        label: fileCommands[id][0],
        shortcut: fileCommands[id][1],
        disabled: !commands?.enabled(id),
        run: () => commands?.dispatch(id),
      })),
    },
    {
      label: "查看",
      accessKey: "v",
      items: [
        {
          label: "刷新当前页",
          shortcut: "F5",
          disabled: !refreshCurrent,
          run: () => refreshCurrent?.(),
        },
        ...(commands?.views ?? []),
      ],
    },
    {
      label: "工具",
      accessKey: "t",
      items: [
        ...(panels?.tasks
          ? [{ label: "任务", run: () => openPage("tasks") }]
          : []),
        ...(panels?.settings
          ? [{ label: "设置", run: () => openPage("settings") }]
          : []),
        ...(panels?.development
          ? [{ label: "开发验证", run: () => openPage("development") }]
          : []),
      ],
    },
  ];
  return (
    <div className="file-workspace" ref={root}>
      <WindowChrome>
        <div
          className="workspace-tabs"
          role="tablist"
          aria-label="工作区标签页"
        >
          {tabs.map((tab) => (
            <div
              className={`workspace-tab ${active === tab.id ? "is-active" : ""}`}
              key={tab.id}
            >
              <button
                role="tab"
                aria-selected={active === tab.id}
                aria-controls={`file-tab-${tab.id}`}
                onClick={() => {
                  if (tab.id === 0 && active !== 0) returnTo.current = active;
                  setActive(tab.id);
                }}
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
          ))}
          <button
            className="tab-new"
            aria-label="新建浏览标签页"
            title="新建浏览标签页 (Ctrl+T)"
            onClick={add}
          >
            +
          </button>
        </div>
      </WindowChrome>
      <AppMenu groups={groups} />
      <dialog
        ref={partitionPicker}
        className="asset-dialog"
        aria-labelledby="browse-partition-title"
      >
        <h2 id="browse-partition-title">切换浏览分区</h2>
        <ul className="browse-partitions">
          {assets.map(({ registration }) => (
            <li key={registration.partition_id}>
              <button
                onClick={() => {
                  partitionPicker.current?.close();
                  choosePartition(active, registration.partition_id);
                }}
              >
                {registration.name}
              </button>
            </li>
          ))}
        </ul>
        <form method="dialog">
          <button>取消</button>
        </form>
      </dialog>
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
                  indexVersion={completedTasks
                    .filter((t) => t.partition_id === tab.partition)
                    .map((t) => `${t.id}:${t.updated}`)
                    .sort()
                    .join("|")}
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
                    setSearchContext(
                      page && tab.partition
                        ? {
                            partition: tab.partition,
                            entry: page.location,
                            name: tab.title,
                          }
                        : undefined,
                    );
                    search(text);
                  }}
                  onTitle={title}
                  onChoose={(partition) => choosePartition(tab.id, partition)}
                />
              )}
            </section>
          );
        })}
      {tabs.some((tab) => tab.kind === "search") && (
        <section
          role="tabpanel"
          id="file-tab-0"
          aria-label="搜索"
          className="workspace-panel"
          hidden={active !== 0}
        >
          {searchReady ? (
            <Search
              assets={assets}
              partitions={assets.map((a) => ({
                id: a.registration.partition_id,
                name: a.registration.name,
              }))}
              includePath={includePath}
              request={query}
              active={visible && active === 0}
              onCommands={setCommands}
              onLocate={locate}
              categories={categories}
              completedTasks={completedTasks}
              context={searchContext}
            />
          ) : (
            <p className="browse-message" role="status">
              正在读取搜索设置…
            </p>
          )}
        </section>
      )}
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
              className={`workspace-panel ${page === "tasks" ? "tasks-page" : ""}`}
              hidden={active !== tab.id}
            >
              {panels[page]}
            </section>
          );
        })}
    </div>
  );
}
