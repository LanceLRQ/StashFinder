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
import Search from "./Search";
import WindowChrome from "./WindowChrome";
import "./workspace.css";
import type { Category, SearchContext } from "./search-options";
import { readDirectory, type DirectoryPage } from "./browse-api";
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
const tabIcons: Record<Tab["kind"], string> = {
  browse: "M2 5h5l2 2h9v10H2z",
  search: "M14 9a5 5 0 1 1-10 0 5 5 0 0 1 10 0Zm-1 4 5 5",
  overview: "M3 12h14v5H3z M3 12l3-7h8l3 7 M6 14.5h1",
  tasks: "M4 4h12v14H4z M7 8h6 M7 12h6",
  settings: "M3 6h14 M3 14h14 M7 3v6 M13 11v6",
  development: "m7 5-5 5 5 5 m6-10 5 5-5 5",
};
const locationKey = "stashfinder.last-directory";
function savedLocation() {
  try {
    const value = JSON.parse(localStorage.getItem(locationKey) ?? "null");
    if (
      value?.version === 1 &&
      typeof value.partitionId === "string" &&
      typeof value.filesystemId === "string" &&
      Array.isArray(value.entry) &&
      value.entry.length > 0 &&
      value.entry.every(
        (byte: unknown) =>
          typeof byte === "number" &&
          Number.isInteger(byte) &&
          byte >= 0 &&
          byte <= 255,
      )
    ) {
      return value as {
        partitionId: string;
        filesystemId: string;
        entry: number[];
      };
    }
  } catch {
    // Local storage may be unavailable or contain an old record.
  }
  return null;
}
export default function Workspace({
  assets,
  includePath,
  request,
  browseRequest,
  categories,
  completedTasks = [],
  automatic,
  paused = [],
  visible = true,
  searchReady = true,
  startupReady,
  device,
  panels,
  notice,
}: {
  assets: Asset[];
  includePath: boolean;
  request: { text: string; revision: number };
  browseRequest?: { partition: string; revision: number };
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
  panels?: Partial<Record<AuxiliaryPage, ReactNode>>;
  notice?: ReactNode;
}) {
  const [tabs, setTabs] = useState<Tab[]>([newTab(1)]);
  const [active, setActive] = useState(1);
  const [query, setQuery] = useState({ text: "", revision: 0 });
  const sequence = useRef(1);
  const root = useRef<HTMLDivElement>(null);
  const returnTo = useRef(1);
  const locations = useRef(new Map<number, DirectoryPage>());
  const searchFocus = useRef(false);
  const startupDone = useRef(false);
  const startup = useRef<Pick<
    BrowseTab,
    "partition" | "filesystem" | "parent" | "title"
  > | null>(null);
  const [searchContext, setSearchContext] = useState<SearchContext>();
  const [commands, setCommands] = useState<FileCommandContext | null>(null);
  const location = useCallback(
    (id: number, page: DirectoryPage, partition: string, isActive: boolean) => {
      locations.current.set(id, page);
      if (!isActive || startupReady === undefined) return;
      const entry = page.location;
      startup.current = {
        partition,
        filesystem: page.filesystem_id,
        parent: entry,
        title: page.ancestors.at(-1)?.name || "浏览",
      };
      try {
        localStorage.setItem(
          locationKey,
          JSON.stringify({
            version: 1,
            partitionId: partition,
            filesystemId: page.filesystem_id,
            entry,
          }),
        );
      } catch {
        // Browsing remains available when local storage is blocked.
      }
    },
    [startupReady !== undefined],
  );
  const activeRef = useRef(active);
  activeRef.current = active;
  const search = useCallback((text: string) => {
    if (activeRef.current > 0) returnTo.current = activeRef.current;
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
    startupDone.current = true;
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
    if (!startupReady || startupDone.current || !assets.length) return;
    let current = true;
    const saved = savedLocation();
    const eligible = assets.filter((a) =>
      ["online", "offline", "historical"].includes(a.connection),
    );
    const matching = eligible.find(
      (a) =>
        a.registration.partition_id === saved?.partitionId &&
        a.registration.filesystem_id === saved.filesystemId,
    );
    const ordered = [
      ...new Set([
        ...(matching ? [matching] : []),
        ...eligible.filter((a) => a.connection === "online"),
        ...eligible,
      ]),
    ];
    async function restore() {
      let chosen = ordered[0] || assets[0];
      let parent = chosen === matching ? saved!.entry : null;
      // Probe alternate partitions only when there is a choice. The pane presents the real index error if none are readable.
      for (const asset of ordered.length > 1 ? ordered : []) {
        const { partition_id, filesystem_id } = asset.registration;
        const entry = asset === matching ? saved!.entry : null;
        const read = (parent: number[] | null) =>
          readDirectory({ partition_id, filesystem_id, parent });
        const page = await read(entry).catch((e) =>
          entry && String(e).includes("目录已不存在")
            ? read(null).catch(() => null)
            : null,
        );
        if (!current || startupDone.current) return;
        if (page) {
          chosen = asset;
          parent = entry;
          break;
        }
      }
      if (!current || startupDone.current) return;
      startupDone.current = true;
      const {
        partition_id: partition,
        filesystem_id: filesystem,
        name: title,
      } = chosen.registration;
      startup.current = { partition, filesystem, title, parent };
      setTabs((previous) => {
        const empty = previous.find(
          (tab): tab is BrowseTab => tab.kind === "browse" && !tab.partition,
        );
        return empty
          ? previous.map((tab) =>
              tab.id === empty.id
                ? { ...empty, partition, filesystem, title, parent }
                : tab,
            )
          : previous;
      });
    }
    void restore();
    return () => {
      current = false;
    };
  }, [assets, startupReady]);
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
    locations.current.delete(id);
    const remaining = tabs.filter((t) => t.id !== id);
    if (!remaining.length) {
      const next = ++sequence.current;
      const location = startup.current;
      const valid =
        location &&
        assets.some(
          (asset) =>
            asset.registration.partition_id === location.partition &&
            asset.registration.filesystem_id === location.filesystem,
        );
      setTabs([{ ...newTab(next), ...(valid ? location : {}) }]);
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
  }, [active, tabs, assets, commands]);
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
        { label: "关闭标签页", shortcut: "Ctrl+W", run: () => close(active) },
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
        { label: "概览（硬盘）", run: () => openPage("overview") },
        ...(commands?.views ?? []),
      ],
    },
    {
      label: "工具",
      accessKey: "t",
      items: [
        { label: "任务", run: () => openPage("tasks") },
        { label: "设置", run: () => openPage("settings") },
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
                  if (tab.id === 0 && active > 0) returnTo.current = active;
                  setActive(tab.id);
                }}
                title={tab.title}
              >
                <svg viewBox="0 0 20 20" aria-hidden="true">
                  <path d={tabIcons[tab.kind]} />
                </svg>
                <span>{tab.title}</span>
              </button>
              <button
                className="tab-close"
                aria-label={`关闭${tab.title}标签页`}
                onClick={() => close(tab.id)}
              >
                ×
              </button>
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
      {panels && <AppMenu groups={groups} />}
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
                  onChoose={(partition) => {
                    startupDone.current = true;
                    const chosen = assets.find(
                      (a) => a.registration.partition_id === partition,
                    )!;
                    setTabs((previous) =>
                      previous.map((t) =>
                        t.kind === "browse" && t.id === tab.id
                          ? {
                              ...t,
                              partition,
                              filesystem: chosen.registration.filesystem_id,
                              parent: null,
                              focusEntry: undefined,
                              title: chosen.registration.name,
                            }
                          : t,
                      ),
                    );
                  }}
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
