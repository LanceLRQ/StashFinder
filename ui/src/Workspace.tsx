import { useCallback, useEffect, useRef, useState } from "react";
import type { Asset } from "./asset-api";
import type { SearchHit } from "./index-api";
import BrowsePane from "./BrowsePane";
import Search from "./Search";
import "./workspace.css";
import type { Category, SearchContext } from "./search-options";
import type { DirectoryPage } from "./browse-api";

type Tab = {
  id: number;
  title: string;
  partition: string | null;
  filesystem: string | null;
  parent: number[] | null;
  focusEntry?: number[];
};
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
}) {
  const [tabs, setTabs] = useState<Tab[]>([
    { id: 1, title: "浏览", partition: null, filesystem: null, parent: null },
  ]);
  const [active, setActive] = useState(1);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState({ text: "", revision: 0 });
  const sequence = useRef(1);
  const root = useRef<HTMLDivElement>(null);
  const returnTo = useRef(1);
  const locations = useRef(new Map<number, DirectoryPage>());
  const [searchContext, setSearchContext] = useState<SearchContext>();
  const location = useCallback((id: number, page: DirectoryPage) => {
    locations.current.set(id, page);
  }, []);
  const activeRef = useRef(active);
  activeRef.current = active;
  const search = useCallback((text: string) => {
    if (activeRef.current !== 0) returnTo.current = activeRef.current;
    setQuery((q) => ({ text, revision: q.revision + 1 }));
    setSearchOpen(true);
    setActive(0);
  }, []);
  useEffect(() => {
    if (request.revision > 0) search(request.text);
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
    const empty = tabs.find((t) => t.id === activeRef.current && !t.partition);
    const id = empty?.id ?? ++sequence.current;
    const tab: Tab = {
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
  const title = useCallback(
    (id: number, value: string) =>
      setTabs((previous) =>
        previous.some((t) => t.id === id && t.title !== value)
          ? previous.map((t) => (t.id === id ? { ...t, title: value } : t))
          : previous,
      ),
    [],
  );
  function add() {
    const id = ++sequence.current;
    setTabs((previous) => [
      ...previous,
      { id, title: "浏览", partition: null, filesystem: null, parent: null },
    ]);
    setActive(id);
  }
  function close(id: number) {
    locations.current.delete(id);
    if (id === 0) {
      setSearchOpen(false);
      if (active === 0)
        setActive(
          tabs.some((t) => t.id === returnTo.current)
            ? returnTo.current
            : tabs[tabs.length - 1].id,
        );
      return;
    }
    const remaining = tabs.filter((t) => t.id !== id);
    if (!remaining.length) {
      const next = ++sequence.current;
      setTabs([
        {
          id: next,
          title: "浏览",
          partition: null,
          filesystem: null,
          parent: null,
        },
      ]);
      if (active === id) setActive(next);
    } else {
      setTabs(remaining);
      if (active === id)
        setActive(
          remaining[
            Math.min(
              tabs.findIndex((t) => t.id === id),
              remaining.length - 1,
            )
          ].id,
        );
    }
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
      if (!root.current || root.current.closest("[hidden]") || e.isComposing)
        return;
      if (e.ctrlKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        const input = root.current.querySelector(
          'section:not([hidden]) input[type="search"]',
        ) as HTMLInputElement | null;
        input?.focus();
        input?.select();
      }
      if (e.ctrlKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        close(active);
      }
      if (e.ctrlKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        add();
      }
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        const ids = [...tabs.map((t) => t.id), ...(searchOpen ? [0] : [])];
        setActive(
          ids[
            (ids.indexOf(active) + (e.shiftKey ? ids.length - 1 : 1)) %
              ids.length
          ],
        );
      }
    }
    document.addEventListener("keydown", keyDown);
    return () => document.removeEventListener("keydown", keyDown);
  }, [active, tabs, searchOpen]);
  return (
    <div className="file-workspace" ref={root}>
      <div className="workspace-tabs" role="tablist" aria-label="文件标签页">
        {[...tabs, ...(searchOpen ? [{ id: 0, title: "搜索" }] : [])].map(
          (tab) => (
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
              >
                {tab.title}
              </button>
              <button
                className="tab-close"
                aria-label={`关闭${tab.title}标签页`}
                onClick={() => close(tab.id)}
              >
                ×
              </button>
            </div>
          ),
        )}
        <button
          className="tab-new"
          aria-label="新建浏览标签页"
          title="新建浏览标签页 (Ctrl+T)"
          onClick={add}
        >
          +
        </button>
      </div>
      {tabs.map((tab) => {
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
                key={`${tab.id}:${tab.filesystem}`}
                id={tab.id}
                asset={asset}
                assets={assets}
                initialParent={tab.parent}
                focusEntry={tab.focusEntry}
                onLocation={location}
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
                  const chosen = assets.find(
                    (a) => a.registration.partition_id === partition,
                  )!;
                  setTabs((previous) =>
                    previous.map((t) =>
                      t.id === tab.id
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
      {searchOpen && (
        <section
          role="tabpanel"
          id="file-tab-0"
          aria-label="搜索"
          className="workspace-panel"
          hidden={active !== 0}
        >
          <Search
            assets={assets}
            partitions={assets.map((a) => ({
              id: a.registration.partition_id,
              name: a.registration.name,
            }))}
            includePath={includePath}
            request={query}
            active={visible && active === 0}
            onLocate={locate}
            categories={categories}
            indexVersion={completedTasks
              .map((t) => `${t.id}:${t.updated}`)
              .sort()
              .join("|")}
            context={searchContext}
          />
        </section>
      )}
    </div>
  );
}
