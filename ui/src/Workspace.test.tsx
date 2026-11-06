import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import Workspace from "./Workspace";
import FileOperations from "./FileOperations";
import DirectoryTree from "./DirectoryTree";
import { readDirectory, type DirectoryPage } from "./browse-api";
import type { Asset } from "./asset-api";
import { invoke } from "@tauri-apps/api/core";

vi.mock("./browse-api", async (original) => ({
  ...(await original<object>()),
  readDirectory: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});
const asset = {
  registration: {
    partition_id: "p",
    filesystem_id: "fs",
    name: "Movies",
    state: "ready",
  },
  connection: "offline",
  mounts: [],
} as unknown as Asset;
const row = (id: number, name: string, kind = "file") => ({
  entry: [id],
  object: [id],
  parent: [1],
  name,
  path: name,
  kind,
  size: null,
  created: null,
  modified: null,
  attributes: 0,
});
const root: DirectoryPage = {
  filesystem_id: "fs",
  generation: "1",
  revision: "2",
  location: [1],
  ancestors: [{ entry: [1], name: "\\" }],
  entries: [row(2, "Films", "folder"), row(3, "notes.txt")],
  next: null,
  partial: false,
  indexed_at: 1700000000,
  needs_validation: false,
};
function setup() {
  vi.mocked(readDirectory).mockImplementation(async (r) => {
    const page =
      r.parent?.[0] === 2
        ? {
            ...root,
            location: [2],
            ancestors: [...root.ancestors, { entry: [2], name: "Films" }],
            entries: [row(4, "movie.mkv")],
          }
        : root;
    return {
      ...page,
      entries: r.folders_only
        ? page.entries.filter((x) => x.kind === "folder")
        : page.entries,
    };
  });
  vi.mocked(invoke).mockImplementation(async (_, args: any) =>
    args.action.search_start
      ? { kind: "search_started", data: 1 }
      : args.action.search_batch
        ? {
            kind: "search_page",
            data: {
              done: true,
              row: null,
              rows: [],
              total: 0,
              groups: 0,
              missing: 0,
              next: null,
              more: false,
              unavailable: [],
              error: null,
            },
          }
        : { kind: "accepted" },
  );
  return render(
    <Workspace
      assets={[asset]}
      includePath
      request={{ text: "", revision: 0 }}
    />,
  );
}
test("published index changes refresh the current directory without changing its location", async () => {
  const view = setup();
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  fireEvent.doubleClick(await screen.findByRole("row", { name: /Films/ }));
  await screen.findByRole("row", { name: /movie.mkv/ });
  vi.mocked(readDirectory).mockClear();
  view.rerender(
    <Workspace
      assets={[asset]}
      includePath
      request={{ text: "", revision: 0 }}
      completedTasks={[{ id: "completed", partition_id: "p", updated: 2 }]}
    />,
  );
  await waitFor(() =>
    expect(
      vi
        .mocked(readDirectory)
        .mock.calls.some(([r]) => r.parent?.[0] === 2 && !r.folders_only),
    ).toBe(true),
  );
  expect(
    screen.getByRole("tab", { name: "Films" }).getAttribute("aria-selected"),
  ).toBe("true");
});

test("keyboard paste keeps pending-index feedback when automatic updates are disabled", async () => {
  vi.mocked(readDirectory).mockResolvedValue(root);
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "file_paste"
      ? "完成 1 项"
      : command === "file_report"
        ? {
            id: 1,
            total: 1,
            completed: 1,
            rows: [
              {
                index: 0,
                source: "new.txt",
                state: "completed",
                destination: "Movies\\new.txt",
                error: null,
                note: null,
              },
            ],
            next: null,
          }
        : undefined,
  );
  render(
    <FileOperations>
      <Workspace
        assets={[{ ...asset, connection: "online" }]}
        includePath
        request={{ text: "", revision: 0 }}
        automatic={false}
      />
    </FileOperations>,
  );
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  await screen.findByRole("row", { name: /notes.txt/ });
  fireEvent.keyDown(screen.getByLabelText("目录文件列表"), {
    key: "v",
    ctrlKey: true,
  });
  expect(await screen.findByText("文件操作已结束，索引待更新")).toBeTruthy();
  expect(screen.getByRole("row", { name: /notes.txt/ })).toBeTruthy();
});
test("keeps independent directory selection and scroll while a single search tab is reused", async () => {
  setup();
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  const first = await screen.findByRole("row", { name: /Films/ });
  fireEvent.doubleClick(first);
  const file = await screen.findByRole("row", { name: /movie.mkv/ });
  fireEvent.click(file);
  const list = within(screen.getByRole("tabpanel")).getByLabelText(
    "目录文件列表",
  );
  list.scrollTop = 123;
  fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  await screen.findByRole("row", { name: /notes.txt/ });
  fireEvent.click(screen.getByRole("tab", { name: "Films" }));
  expect(
    screen
      .getByRole("row", { name: /movie.mkv/ })
      .getAttribute("aria-selected"),
  ).toBe("true");
  expect(
    within(screen.getByRole("tabpanel")).getByLabelText("目录文件列表")
      .scrollTop,
  ).toBe(123);
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "movie" },
  });
  await screen.findByText("没有匹配的文件");
  fireEvent.click(screen.getByRole("tab", { name: "Films" }));
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "other" },
  });
  expect(screen.getAllByRole("tab", { name: "搜索" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "关闭搜索标签页" }));
  expect(
    screen.getByRole("tab", { name: "Films" }).getAttribute("aria-selected"),
  ).toBe("true");
  expect(
    within(screen.getByRole("tabpanel")).getByLabelText("目录文件列表")
      .scrollTop,
  ).toBe(123);
});
test("rejects stale pagination without discarding visible rows and refreshes the same directory", async () => {
  setup();
  vi.mocked(readDirectory).mockImplementation(async (r) => {
    if (r.offset) throw new Error("目录索引已更新，请刷新");
    return { ...root, next: r.folders_only ? null : 2 };
  });
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  fireEvent.click(await screen.findByRole("button", { name: "加载更多文件" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "目录索引已更新，请刷新",
  );
  expect(screen.getByRole("row", { name: /notes.txt/ })).toBeTruthy();
  expect(readDirectory).toHaveBeenCalledWith(
    expect.objectContaining({ generation: "1", revision: "2", offset: 2 }),
  );
  fireEvent.click(screen.getByRole("button", { name: "刷新目录" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});
test("ignores late directory responses and navigates to the nearest surviving ancestor", async () => {
  setup();
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  fireEvent.doubleClick(await screen.findByRole("row", { name: /Films/ }));
  await screen.findByRole("row", { name: /movie.mkv/ });
  let finish!: (p: DirectoryPage) => void;
  vi.mocked(readDirectory).mockImplementation(async (r) =>
    r.parent?.[0] === 2
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : root,
  );
  fireEvent.click(screen.getByRole("button", { name: "刷新目录" }));
  await waitFor(() => expect(finish).toBeTruthy());
  fireEvent.click(screen.getByRole("button", { name: "上一级" }));
  await screen.findByRole("row", { name: /notes.txt/ });
  await act(async () => finish({ ...root, entries: [row(9, "late.txt")] }));
  expect(screen.queryByText("late.txt")).toBeNull();
  vi.mocked(readDirectory).mockImplementation(async (r) => {
    if (r.parent?.[0] === 2) throw new Error("目录已不存在，请返回上级目录");
    return root;
  });
  fireEvent.doubleClick(screen.getByRole("row", { name: /Films/ }));
  await screen.findByText(/目录已不存在，已返回/);
  expect(screen.getByRole("row", { name: /notes.txt/ })).toBeTruthy();
});
test("shows offline metadata and allows panes and breadcrumbs to be hidden", async () => {
  setup();
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  fireEvent.click(await screen.findByRole("row", { name: /notes.txt/ }));
  const preview = screen.getByRole("complementary", { name: "预览窗格" });
  expect(within(preview).getByText("离线")).toBeTruthy();
  expect(within(preview).getAllByText("未知").length).toBeGreaterThan(0);
  fireEvent.click(screen.getByText("视图"));
  fireEvent.click(screen.getByRole("checkbox", { name: "预览窗格" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "面包屑" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "目录树" }));
  expect(screen.queryByRole("complementary", { name: "预览窗格" })).toBeNull();
  expect(screen.queryByRole("navigation", { name: "目录位置" })).toBeNull();
  expect(screen.queryByRole("navigation", { name: "目录树" })).toBeNull();
});
test("keeps the search field mounted during IME composition", async () => {
  setup();
  const input = screen.getByRole("searchbox");
  fireEvent.compositionStart(input);
  fireEvent.change(input, { target: { value: "电影" } });
  expect(screen.queryByRole("tab", { name: "搜索" })).toBeNull();
  fireEvent.compositionEnd(input, { data: "电影" });
  expect(await screen.findByRole("tab", { name: "搜索" })).toBeTruthy();
  expect(screen.getByRole("searchbox")).toHaveProperty("value", "电影");
});

test("keeps multi-selection and location when the partition name and drive letter change", async () => {
  const view = setup();
  fireEvent.change(screen.getByRole("combobox", { name: "浏览分区" }), {
    target: { value: "p" },
  });
  fireEvent.click(await screen.findByRole("row", { name: /Films/ }));
  fireEvent.click(screen.getByRole("row", { name: /notes.txt/ }), {
    ctrlKey: true,
  });
  expect(screen.getAllByRole("row", { selected: true })).toHaveLength(2);
  view.rerender(
    <Workspace
      assets={[
        {
          ...asset,
          connection: "online",
          mounts: ["Z:\\"],
          registration: { ...asset.registration, name: "Renamed" },
        },
      ]}
      includePath
      request={{ text: "", revision: 0 }}
    />,
  );
  expect(await screen.findByRole("tab", { name: "Renamed" })).toBeTruthy();
  expect(screen.getAllByRole("row", { selected: true })).toHaveLength(2);
  expect(screen.getByText(/Z:/)).toBeTruthy();
});

test("does not append an old tree page after an index refresh", async () => {
  let finish!: (page: DirectoryPage) => void;
  vi.mocked(readDirectory).mockImplementation(async (r) =>
    r.offset
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : { ...root, entries: [row(2, "OldFolder", "folder")], next: 1 },
  );
  const view = render(
    <DirectoryTree
      partition="p"
      filesystem="fs"
      name="Movies"
      selected={null}
      version="1"
      onNavigate={() => {}}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "更多目录" }));
  await waitFor(() => expect(finish).toBeTruthy());
  vi.mocked(readDirectory).mockResolvedValue({
    ...root,
    revision: "3",
    entries: [row(6, "NewFolder", "folder")],
    next: null,
  });
  view.rerender(
    <DirectoryTree
      partition="p"
      filesystem="fs"
      name="Movies"
      selected={null}
      version="2"
      onNavigate={() => {}}
    />,
  );
  await screen.findByRole("button", { name: "NewFolder" });
  await act(async () =>
    finish({ ...root, entries: [row(8, "LateFolder", "folder")] }),
  );
  expect(screen.queryByRole("button", { name: "LateFolder" })).toBeNull();
  expect(screen.getByRole("button", { name: "NewFolder" })).toBeTruthy();
});

test("handles workspace shortcuts when focus is outside a tab", () => {
  setup();
  fireEvent.keyDown(document, { ctrlKey: true, key: "f" });
  expect(document.activeElement).toBe(screen.getByRole("searchbox"));
  fireEvent.keyDown(document, { ctrlKey: true, key: "t" });
  expect(screen.getAllByRole("tab")).toHaveLength(2);
  fireEvent.keyDown(document, { ctrlKey: true, key: "w" });
  expect(screen.getAllByRole("tab")).toHaveLength(1);
});

test("uses the empty browse tab for an overview partition request", async () => {
  const view = setup();
  view.rerender(
    <Workspace
      assets={[asset]}
      includePath
      request={{ text: "", revision: 0 }}
      browseRequest={{ partition: "p", revision: 1 }}
    />,
  );
  await screen.findByRole("row", { name: /notes.txt/ });
  expect(screen.getAllByRole("tab")).toHaveLength(1);
});
