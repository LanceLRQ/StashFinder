import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeAll, expect, test, vi } from "vitest";
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
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
});
afterEach(() => {
  cleanup();
  localStorage.clear();
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

function choosePartition(name = "Movies") {
  const tree = screen.getByRole("navigation", { name: "目录树" });
  fireEvent.click(within(tree).getByRole("button", { name }));
}

test("only shows tab scroll buttons when tabs overflow and keeps Home available", () => {
  render(
    <Workspace
      assets={[]}
      includePath
      request={{ text: "", revision: 0 }}
      panels={{ overview: <p>概览</p> }}
    />,
  );
  const next = () => screen.queryByRole("button", { name: "向右滚动标签页" });
  expect(next()).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
  const tab = screen.getByRole("tab", { name: "浏览", selected: true });
  const viewport = tab.closest<HTMLElement>(".workspace-tab-viewport")!;
  Object.defineProperties(viewport, {
    clientWidth: { configurable: true, value: 160 },
    scrollWidth: { configurable: true, value: 400 },
    scrollBy: {
      value: ({ left }: ScrollToOptions) => {
        viewport.scrollLeft += left ?? 0;
        fireEvent.scroll(viewport);
      },
    },
  });
  fireEvent.resize(window);
  const previous = screen.getByRole("button", {
    name: "向左滚动标签页",
  }) as HTMLButtonElement;
  expect(previous.disabled).toBe(true);
  fireEvent.click(next()!);
  expect(viewport.scrollLeft).toBe(160);
  expect(previous.disabled).toBe(false);
  fireEvent.click(previous);
  expect(viewport.scrollLeft).toBe(0);
  expect(viewport.contains(screen.getByRole("tab", { name: "概览" }))).toBe(
    false,
  );
  Object.defineProperty(viewport, "clientWidth", { value: 500 });
  fireEvent.resize(window);
  expect(next()).toBeNull();
  expect(previous.isConnected).toBe(false);
});

test("pins Home first, opens a partition once and returns to Home when it closes", async () => {
  const view = setup();
  const props = {
    assets: [asset],
    includePath: true,
    request: { text: "", revision: 0 },
    startupReady: true,
    panels: { overview: <p>概览面板</p> },
  };
  view.unmount();
  const home = render(<Workspace {...props} />);
  expect(screen.getByRole("tab", { name: "概览", selected: true })).toBe(
    screen.getAllByRole("tab")[0],
  );
  fireEvent.keyDown(document, { key: "w", ctrlKey: true });
  expect(screen.queryByRole("button", { name: "关闭概览标签页" })).toBeNull();
  expect(readDirectory).not.toHaveBeenCalled();
  home.rerender(
    <Workspace {...props} browseRequest={{ partition: "p", revision: 1 }} />,
  );
  await screen.findByRole("row", { name: /notes.txt/ });
  home.rerender(
    <Workspace {...props} browseRequest={{ partition: "p", revision: 2 }} />,
  );
  expect(screen.getAllByRole("tab")).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "关闭Movies标签页" }));
  expect(
    screen.getByRole("tab", { name: "概览", selected: true }),
  ).toBeTruthy();
});

test.each(["尚无目录索引", "目录索引尚未完成"])(
  "keeps the partition collapsed and shows one loading state while %s",
  async (message) => {
    const view = setup();
    const read = vi.mocked(readDirectory).getMockImplementation()!;
    vi.mocked(readDirectory).mockRejectedValue(new Error(message));
    expect(screen.queryByText("正在加载目录索引…")).toBeNull();

    await act(async () => choosePartition());
    const tree = within(screen.getByRole("navigation", { name: "目录树" }));
    const expander = tree.getByRole("button", { name: "展开Movies" });
    expect(expander).toHaveProperty("disabled", true);
    expect(expander.getAttribute("aria-expanded")).toBe("false");
    const waiting = await screen.findByText("正在加载目录索引…");
    expect(waiting.closest(".directory-main")).not.toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(message)).toBeNull();
    expect(
      vi.mocked(readDirectory).mock.calls.some(([r]) => r.folders_only),
    ).toBe(false);

    vi.mocked(readDirectory).mockImplementation(read);
    view.rerender(
      <Workspace
        assets={[asset]}
        includePath
        request={{ text: "", revision: 0 }}
        completedTasks={[{ id: "build", partition_id: "p", updated: 1 }]}
      />,
    );
    await screen.findByRole("row", { name: /notes.txt/ });
    expect(await tree.findByRole("button", { name: "Films" })).toBeTruthy();
    expect(screen.queryByText("正在加载目录索引…")).toBeNull();
  },
);

test("lists every partition in a new tab and expands the chosen partition from the tree", async () => {
  const view = setup();
  const backup = {
    ...asset,
    registration: {
      ...asset.registration,
      partition_id: "backup",
      name: "Backup",
    },
  };
  view.rerender(
    <Workspace
      assets={[asset, backup]}
      includePath
      request={{ text: "", revision: 0 }}
      startupReady
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
  const tree = within(screen.getByRole("navigation", { name: "目录树" }));
  for (const name of ["Movies", "Backup"]) {
    expect(
      tree.getByRole("button", { name }).querySelector("svg"),
    ).toBeTruthy();
    expect(
      tree
        .getByRole("button", { name: `展开${name}` })
        .getAttribute("aria-expanded"),
    ).toBe("false");
  }
  expect(screen.queryByRole("combobox", { name: "浏览分区" })).toBeNull();
  fireEvent.click(tree.getByRole("button", { name: "展开Backup" }));
  const opened = within(screen.getByRole("navigation", { name: "目录树" }));
  const folder = await opened.findByRole("button", { name: "Films" });
  const folderShape = folder.querySelector("svg path")?.getAttribute("d");
  expect(folderShape).toBeTruthy();
  expect(folderShape).not.toBe(
    opened
      .getByRole("button", { name: "Backup" })
      .querySelector("svg path")
      ?.getAttribute("d"),
  );
  const listed = await screen.findByRole("row", { name: /Films/ });
  expect(listed.querySelector(".file-icon svg path")?.getAttribute("d")).toBe(
    folderShape,
  );
  expect(
    opened.getByRole("button", { name: "收起Backup" }).querySelector("svg"),
  ).toBeTruthy();
  fireEvent.click(folder);
  await screen.findByRole("row", { name: /movie.mkv/ });
  const breadcrumb = within(
    screen.getByRole("navigation", { name: "目录位置" }),
  );
  fireEvent.click(breadcrumb.getByRole("button", { name: "Backup" }));
  await screen.findByRole("row", { name: /notes.txt/ });
  fireEvent.click(opened.getByRole("button", { name: "收起Backup" }));
  expect(opened.queryByRole("button", { name: "Films" })).toBeNull();
});

test("directory details uses the column menu and requests both sort directions", async () => {
  setup();
  choosePartition();
  await screen.findByRole("row", { name: /notes.txt/ });
  fireEvent.contextMenu(screen.getByRole("button", { name: "按名称排序" }));
  const menu = screen.getByRole("menu", { name: "详情列" });
  expect(
    within(menu)
      .getByRole("menuitemcheckbox", { name: "名称", checked: true })
      .hasAttribute("disabled"),
  ).toBe(true);
  fireEvent.click(
    within(menu).getByRole("menuitemcheckbox", {
      name: "创建时间",
      checked: false,
    }),
  );
  const header = screen.getByRole("button", { name: "按创建时间排序" });
  for (const descending of [false, true]) {
    fireEvent.click(header);
    await waitFor(() =>
      expect(readDirectory).toHaveBeenLastCalledWith(
        expect.objectContaining({ sort: { field: "created", descending } }),
      ),
    );
    expect(header.closest("th")?.getAttribute("aria-sort")).toBe(
      descending ? "descending" : "ascending",
    );
  }
  expect(
    localStorage.getItem("stashfinder.details.directory-list.v1"),
  ).toContain("created");
});

test("directory thumbnail modes preserve focus, selection and the tab's layout", async () => {
  setup();
  choosePartition();
  const row = await screen.findByRole("row", { name: /notes.txt/ });
  row.focus();
  fireEvent.click(row);
  const menu = screen.getByRole("button", { name: "查看" });
  const choose = (name: string) => {
    fireEvent.pointerDown(menu);
    fireEvent.click(menu);
    fireEvent.click(screen.getByRole("menuitemradio", { name }));
  };
  choose("缩略图（小图标）");
  const item = screen.getByRole("option", { name: "notes.txt" });
  expect(item.getAttribute("aria-selected")).toBe("true");
  expect(document.activeElement).toBe(item);
  choose("缩略图（中图标）");
  choose("缩略图（大图标）");
  expect(document.activeElement).toBe(
    screen.getByRole("option", { name: "notes.txt" }),
  );
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "notes" },
  });
  await screen.findByRole("tab", { name: "搜索" });
  fireEvent.click(menu);
  expect(
    screen
      .getByRole("menuitemradio", { name: "详情" })
      .getAttribute("aria-checked"),
  ).toBe("true");
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  fireEvent.click(screen.getByRole("tab", { name: "Movies" }));
  expect(
    screen
      .getByRole("option", { name: "notes.txt" })
      .getAttribute("aria-selected"),
  ).toBe("true");
  choose("详情");
  expect(
    screen
      .getByRole("row", { name: /notes.txt/ })
      .getAttribute("aria-selected"),
  ).toBe("true");
}, 15000);
test("refreshes the current directory once from either workspace entry", async () => {
  setup();
  choosePartition();
  const row = await screen.findByRole("row", { name: /notes.txt/ });
  const reads = () =>
    vi
      .mocked(readDirectory)
      .mock.calls.filter(([request]) => !request.folders_only).length;
  const initial = reads();
  fireEvent.click(screen.getByRole("button", { name: "查看" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "刷新当前页" }));
  await waitFor(() => expect(reads()).toBe(initial + 1));
  fireEvent.keyDown(row, { key: "F5" });
  await waitFor(() => expect(reads()).toBe(initial + 2));
  expect(screen.queryByRole("button", { name: "刷新目录" })).toBeNull();
  expect(screen.queryByText("视图")).toBeNull();
});

test("keeps partition switching reachable when the tree and breadcrumbs are hidden", async () => {
  const view = setup();
  const backup = {
    ...asset,
    registration: {
      ...asset.registration,
      partition_id: "backup",
      name: "Backup",
    },
  };
  view.rerender(
    <Workspace
      assets={[asset, backup]}
      includePath
      request={{ text: "", revision: 0 }}
    />,
  );
  choosePartition();
  await screen.findByRole("row", { name: /notes.txt/ });
  expect(screen.queryByRole("combobox", { name: "浏览分区" })).toBeNull();
  expect(screen.queryByRole("complementary", { name: "预览窗格" })).toBeNull();
  for (const name of ["目录树", "面包屑"]) {
    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name }));
  }
  expect(screen.queryByRole("combobox", { name: "浏览分区" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "文件" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "切换浏览分区…" }));
  const dialog = screen.getByRole("dialog", { name: "切换浏览分区" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Backup" }));
  await screen.findByRole("tab", { name: "Backup", selected: true });
  expect(readDirectory).toHaveBeenCalledWith(
    expect.objectContaining({ partition_id: "backup" }),
  );
});

test("published index changes refresh the current directory without changing its location", async () => {
  const view = setup();
  choosePartition();
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

test.each(["online", "offline"] as const)(
  "routes Edit, context menu and keyboard to the same %s selection",
  async (connection) => {
    vi.mocked(readDirectory).mockResolvedValue(root);
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "file_prepare")
        return {
          id: 7,
          count: 2,
          names: ["Films", "notes.txt"],
          same_parent: true,
          single_file: false,
        };
      if (command === "file_report")
        return { id: 7, total: 2, completed: 2, rows: [], next: null };
      return "已复制";
    });
    render(
      <FileOperations>
        <Workspace
          assets={[{ ...asset, connection }]}
          includePath
          request={{ text: "", revision: 0 }}
          panels={{ overview: <p>概览面板</p> }}
        />
      </FileOperations>,
    );
    const edit = screen.getByRole("button", { name: "编辑" });
    fireEvent.click(edit);
    expect(screen.getByRole("menuitem", { name: "复制路径" })).toHaveProperty(
      "ariaDisabled",
      "true",
    );
    expect(screen.getByRole("menuitem", { name: "粘贴" })).toHaveProperty(
      "ariaDisabled",
      "true",
    );
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
    choosePartition();
    const notes = await screen.findByRole("row", { name: /notes.txt/ });
    const films = screen.getByRole("row", { name: /Films/ });
    const list = screen.getByLabelText("目录文件列表");
    fireEvent.click(films);
    fireEvent.click(notes, { ctrlKey: true });
    const selection = {
      origin: {
        kind: "directory",
        context: {
          partition_id: "p",
          filesystem_id: "fs",
          generation: "1",
          revision: "2",
          entry: [1],
        },
        all: false,
      },
      entries: [[2], [3]],
      positions: [],
    };
    for (const source of ["edit", "context", "keyboard"]) {
      if (source === "edit") {
        fireEvent.click(edit);
        expect(
          screen
            .getByRole("menuitem", { name: "复制文件" })
            .getAttribute("aria-disabled"),
        ).toBe(String(connection === "offline"));
        expect(
          screen
            .getByRole("menuitem", { name: "重命名" })
            .getAttribute("aria-disabled"),
        ).toBe("true");
        expect(
          screen
            .getByRole("menuitem", { name: "粘贴" })
            .getAttribute("aria-disabled"),
        ).toBe(String(connection === "offline"));
        fireEvent.click(screen.getByRole("menuitem", { name: "复制路径" }));
      } else if (source === "context") {
        fireEvent.contextMenu(notes);
        expect(
          screen.getByRole("menuitem", { name: "复制文件" }),
        ).toHaveProperty("disabled", connection === "offline");
        fireEvent.click(screen.getByRole("menuitem", { name: "复制路径" }));
      } else {
        fireEvent.keyDown(list, { key: "C", ctrlKey: true, shiftKey: true });
      }
      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("file_run", {
          id: 7,
          action: "copy_path",
        }),
      );
      expect(invoke).toHaveBeenCalledWith("file_prepare", {
        input: selection,
        recordsOnly: true,
      });
      vi.mocked(invoke).mockClear();
    }
    const shortcut = { key: "C", ctrlKey: true, shiftKey: true };
    expect(fireEvent.keyDown(screen.getByRole("searchbox"), shortcut)).toBe(
      true,
    );
    expect(fireEvent.keyDown(list, { ...shortcut, isComposing: true })).toBe(
      true,
    );
    expect(fireEvent.keyDown(list, { ...shortcut, keyCode: 229 })).toBe(true);
    expect(invoke).not.toHaveBeenCalledWith("file_prepare", expect.anything());
    fireEvent.click(screen.getByRole("tab", { name: "概览" }));
    fireEvent.click(edit);
    expect(screen.getByRole("menuitem", { name: "复制路径" })).toHaveProperty(
      "ariaDisabled",
      "true",
    );
  },
);

test.each(["keyboard", "edit"])(
  "%s paste checks the current published directory even when the index awaits validation",
  async (source) => {
    vi.mocked(readDirectory).mockResolvedValue({
      ...root,
      needs_validation: true,
    });
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
          panels={{ overview: <p>概览面板</p> }}
        />
      </FileOperations>,
    );
    fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
    choosePartition();
    await screen.findByRole("row", { name: /notes.txt/ });
    if (source === "keyboard") {
      fireEvent.keyDown(screen.getByLabelText("目录文件列表"), {
        key: "v",
        ctrlKey: true,
      });
    } else {
      fireEvent.click(screen.getByRole("button", { name: "编辑" }));
      fireEvent.click(screen.getByRole("menuitem", { name: "粘贴" }));
    }
    expect(await screen.findByText("文件操作已结束，索引待更新")).toBeTruthy();
    expect(
      (await screen.findByText("完成 1 项")).closest("footer.browse-status"),
    ).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("file_paste", {
      target: {
        partition_id: "p",
        filesystem_id: "fs",
        generation: "1",
        revision: "2",
        entry: [1],
      },
    });
    expect(screen.getByRole("row", { name: /notes.txt/ })).toBeTruthy();
  },
);
test("keeps independent directory selection and scroll while a single search tab is reused", async () => {
  setup();
  choosePartition();
  const first = await screen.findByRole("row", { name: /Films/ });
  fireEvent.doubleClick(first);
  const file = await screen.findByRole("row", { name: /movie.mkv/ });
  fireEvent.click(file);
  const list = within(screen.getByRole("tabpanel")).getByLabelText(
    "目录文件列表",
  );
  list.scrollTop = 123;
  fireEvent.click(screen.getByRole("button", { name: "新建浏览标签页" }));
  choosePartition();
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
  choosePartition();
  fireEvent.click(await screen.findByRole("button", { name: "加载更多文件" }));
  expect(await screen.findByText("目录索引已更新，请刷新")).toHaveProperty(
    "role",
    "status",
  );
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByRole("row", { name: /notes.txt/ })).toBeTruthy();
  expect(readDirectory).toHaveBeenCalledWith(
    expect.objectContaining({ generation: "1", revision: "2", offset: 2 }),
  );
  fireEvent.keyDown(screen.getByRole("tabpanel"), { key: "F5" });
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});
test("ignores late directory responses and navigates to the nearest surviving ancestor", async () => {
  setup();
  choosePartition();
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
  fireEvent.keyDown(screen.getByRole("tabpanel"), { key: "F5" });
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
  choosePartition();
  fireEvent.click(await screen.findByRole("row", { name: /notes.txt/ }));
  const toggle = (name: string) => {
    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name }));
  };
  toggle("预览窗格");
  const preview = screen.getByRole("complementary", { name: "预览窗格" });
  expect(within(preview).getByText("离线")).toBeTruthy();
  expect(within(preview).getAllByText("未知").length).toBeGreaterThan(0);
  toggle("预览窗格");
  toggle("面包屑");
  toggle("目录树");
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
  choosePartition();
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

test("a saved directory does not override Home when the snapshot arrives", () => {
  localStorage.setItem(
    "stashfinder.last-directory",
    JSON.stringify({
      version: 1,
      partitionId: "p",
      filesystemId: "fs",
      entry: [2],
    }),
  );
  const props = {
    includePath: true,
    request: { text: "", revision: 0 },
    panels: { overview: <p>概览面板</p> },
  };
  const view = render(
    <Workspace {...props} assets={[]} startupReady={false} />,
  );
  view.rerender(<Workspace {...props} assets={[asset]} startupReady />);
  expect(
    screen.getByRole("tab", { name: "概览", selected: true }),
  ).toBeTruthy();
  expect(readDirectory).not.toHaveBeenCalled();
});
