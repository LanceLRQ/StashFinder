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
import { invoke } from "@tauri-apps/api/core";
import Search from "./Search";
import type { SearchHit } from "./index-api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.useRealTimers();
  vi.resetAllMocks();
});
const hit = {
  partition_id: "p",
  filesystem_id: "fs",
  partition_name: "Movies",
  connection: "offline",
  mounts: [],
  generation: 1,
  partial: false,
  needs_validation: false,
  hard_link: false,
  indexed_at: 1700000000,
  file: {
    entry: [2],
    parent: [1],
    object: [1],
    name: "movie.txt",
    path: "aaa\\movie.txt",
    kind: "file",
    attributes: 0,
    size: null,
    created: null,
    modified: null,
  },
};
const page = {
  id: 1,
  done: true,
  row: hit,
  next: null,
  more: true,
  error: null,
  unavailable: [],
  rows: [hit],
  total: 1,
  groups: 0,
  missing: 0,
};
function mockPage(
  value: Omit<typeof page, "row" | "error" | "unavailable"> & {
    row: typeof hit | null;
    error: string | null;
    unavailable: string[];
  },
) {
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch)
      return {
        kind: "search_page",
        data: {
          ...value,
          rows: value.row ? [value.row] : [],
          total: value.row ? 1 : 0,
        },
      };
    return { kind: "accepted" };
  });
}

test("locates an offline result by filesystem and parent identity", async () => {
  mockPage(page);
  const locate = vi.fn();
  render(<Search partitions={[]} onLocate={locate} />);
  fireEvent.contextMenu((await screen.findByText("movie.txt")).closest("tr")!);
  fireEvent.click(
    screen.getByRole("menuitem", { name: "在新标签页浏览所在目录" }),
  );
  expect(locate).toHaveBeenCalledWith(
    expect.objectContaining({
      filesystem_id: "fs",
      file: expect.objectContaining({ entry: [2], parent: [1] }),
    }),
  );
});

test("shows the no-match overlay inside the results area", async () => {
  mockPage({ ...page, row: null, more: false });
  render(<Search partitions={[]} />);
  const message = await screen.findByText("没有匹配的文件");
  expect(message.getAttribute("role")).toBe("status");
  expect(message.closest(".search-body")).toBeTruthy();
});

test("shows offline relative paths, unknown metadata and bounded results without file operations", async () => {
  mockPage(page);
  render(<Search partitions={[]} onBack={() => {}} />);
  expect(await screen.findByText("movie.txt")).toBeTruthy();
  expect(screen.getByText("aaa\\movie.txt")).toBeTruthy();
  expect(screen.getByText("离线")).toBeTruthy();
  expect(screen.getAllByText("未知").length).toBeGreaterThan(0);
  expect(screen.getByText("1 条结果").closest("footer")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "打开" })).toBeNull();
});

test.each([
  [false, "索引待校验"],
  [true, "未完成记录"],
])(
  "shows link type and index state only in enabled detail columns (partial=%s)",
  async (partial, indexState) => {
    mockPage({
      ...page,
      row: { ...hit, hard_link: true, needs_validation: true, partial },
    });
    const view = render(<Search partitions={[]} />);
    const row = within((await screen.findByText("movie.txt")).closest("tr")!);
    expect(row.getByRole("cell", { name: "movie.txt" })).toBeTruthy();
    expect(row.getByRole("cell", { name: "离线" })).toBeTruthy();
    expect(row.queryByText("硬链接")).toBeNull();
    expect(row.queryByText(indexState)).toBeNull();

    for (const [column, value] of [
      ["链接类型", "硬链接"],
      ["索引状态", indexState],
    ]) {
      expect(screen.queryByRole("columnheader", { name: column })).toBeNull();
      fireEvent.contextMenu(screen.getByRole("button", { name: "按名称排序" }));
      const menu = within(screen.getByRole("menu", { name: "详情列" }));
      fireEvent.click(
        menu.getByRole("menuitemcheckbox", { name: column, checked: false }),
      );
      expect(screen.getByRole("columnheader", { name: column })).toBeTruthy();
      expect(row.getByRole("cell", { name: value })).toBeTruthy();
    }

    view.unmount();
    render(<Search partitions={[]} />);
    await screen.findByText("movie.txt");
    expect(screen.getByRole("columnheader", { name: "链接类型" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "索引状态" })).toBeTruthy();
  },
);

test("sends filename and partition filters, and distinguishes errors from empty results", async () => {
  mockPage({ ...page, row: null, more: false, error: "索引不可用" });
  render(
    <Search partitions={[{ id: "p", name: "Movies" }]} onBack={() => {}} />,
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "索引不可用",
  );
  expect(screen.queryByText("没有匹配的文件")).toBeNull();
  const filterButton = screen.getByRole("button", { name: "筛选器" });
  expect(
    screen.queryByRole("button", { name: "筛选器", description: "已启用" }),
  ).toBeNull();
  fireEvent.click(filterButton);
  fireEvent.click(screen.getByRole("checkbox", { name: "仅文件名" }));
  fireEvent.click(screen.getByRole("button", { name: "应用" }));
  expect(
    screen.getByRole("button", { name: "筛选器", description: "已启用" }),
  ).toBe(filterButton);
  expect(filterButton.textContent).toBe("筛选器");
  fireEvent.change(screen.getByRole("combobox", { name: "分区范围" }), {
    target: { value: "p" },
  });
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "aaa bbb" },
  });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_start: expect.objectContaining({
          text: "aaa bbb",
          names_only: true,
          partitions: ["p"],
        }),
      },
    }),
  );
});

test("late pages cannot overwrite a newer query, and previous query is cancelled", async () => {
  let resolveOld!: (v: object) => void;
  let sequence = 0;
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start)
      return { kind: "search_started", data: ++sequence };
    if (args.action.search_batch?.id === 1)
      return new Promise((r) => {
        resolveOld = r;
      });
    if (args.action.search_batch)
      return {
        kind: "search_page",
        data: {
          ...page,
          id: 2,
          rows: [
            {
              ...hit,
              partial: true,
              file: { ...hit.file, name: "new.txt" },
            },
          ],
        },
      };
    return { kind: "accepted" };
  });
  render(<Search partitions={[]} onBack={() => {}} />);
  await waitFor(() => expect(resolveOld).toBeTruthy());
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "new" } });
  expect(await screen.findByText("new.txt")).toBeTruthy();
  fireEvent.contextMenu(screen.getByRole("button", { name: "按名称排序" }));
  fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "索引状态" }));
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: {
      search_start: expect.objectContaining({ text: "new", replaces: 1 }),
    },
  });
  await act(async () => resolveOld({ kind: "search_page", data: page }));
  expect(screen.queryByText("movie.txt")).toBeNull();
  expect(screen.getByText(/未完成记录/)).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: { search_cancel: { id: 1 } },
  });
});

test("missing indexes are visible even when no files match", async () => {
  mockPage({
    ...page,
    row: null,
    more: false,
    unavailable: ["Movies：尚无可搜索索引"],
  });
  render(<Search partitions={[]} onBack={() => {}} />);
  expect(await screen.findByText("Movies：尚无可搜索索引")).toBeTruthy();
  expect(screen.getByText("没有匹配的文件")).toBeTruthy();
});

const publication = (revision: number) => ({
  index_version: `1:${revision}`,
  indexed_at: 1,
});
const tick = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

test("inverts a million-result selection without limiting it to the viewport", async () => {
  const fixture = refreshFixture();
  fixture.total = 1_000_000;
  const onCommands = vi.fn();
  render(<Search partitions={[]} onCommands={onCommands} />);
  await tick(0);
  fireEvent.click(screen.getByText("query-1-0.txt"));
  const invert = () =>
    act(() =>
      onCommands.mock.lastCall?.[0].actions.selection
        .find((item: { label: string }) => item.label === "反选")
        .run(),
    );
  invert();
  expect(screen.getByText("已选择 999,999 项")).toBeTruthy();
  expect(screen.getByText("文件操作最多支持 10,000 项")).toBeTruthy();
  invert();
  expect(screen.getByText("已选择 1 项")).toBeTruthy();
  expect(
    screen
      .getByText("query-1-0.txt")
      .closest("tr")
      ?.getAttribute("aria-selected"),
  ).toBe("true");
});

test("coalesces rapid input and submits immediately on Enter", async () => {
  vi.useFakeTimers();
  mockPage(page);
  render(<Search partitions={[]} initialText="initial" />);
  await tick(0);
  const starts = () =>
    vi.mocked(invoke).mock.calls.flatMap(([, args]) => {
      const start = (args as { action: { search_start?: { text: string } } })
        .action.search_start;
      return start ? [start.text] : [];
    });
  expect(starts()).toEqual(["initial"]);
  const input = screen.getByRole("searchbox");
  fireEvent.change(input, { target: { value: "d" } });
  await tick(40);
  fireEvent.change(input, { target: { value: "dl" } });
  await tick(40);
  fireEvent.change(input, { target: { value: "dll" } });
  await tick(40);
  expect(starts()).toEqual(["initial"]);
  fireEvent.keyDown(input, { key: "Enter" });
  await tick(0);
  expect(starts()).toEqual(["initial", "dll"]);
  await tick(160);
  expect(starts()).toEqual(["initial", "dll"]);
});

test("keeps previous rows visible but inactive until a replacement search completes", async () => {
  vi.useFakeTimers();
  const fixture = refreshFixture();
  render(<Search partitions={[]} />);
  await tick(0);
  const input = screen.getByRole("searchbox");
  const oldRow = screen.getByText("query-1-0.txt").closest("tr")!;
  fireEvent.contextMenu(oldRow);
  expect(screen.getByRole("menu")).toBeTruthy();

  fixture.delay = (id) => (id === 2 ? 400 : 0);
  fireEvent.change(input, { target: { value: "new" } });
  await tick(75);
  expect(screen.getByText("正在搜索…")).toBeTruthy();
  expect(screen.getByText("query-1-0.txt").closest("tr")).toBe(oldRow);
  expect(oldRow.closest("[inert]")).toBeTruthy();
  expect(screen.queryByRole("menu")).toBeNull();
  expect(screen.queryByText(/已选择/)).toBeNull();

  await tick(400);
  expect(screen.queryByText("query-1-0.txt")).toBeNull();
  expect(screen.getByText("query-2-0.txt").closest("[inert]")).toBeNull();
  expect(fixture.cancels).toContain(1);

  fixture.fail = true;
  fireEvent.change(input, { target: { value: "failed" } });
  await tick(75);
  expect(screen.getByRole("alert").textContent).toBe("索引暂时不可用");
  expect(screen.queryByText("query-2-0.txt")).toBeNull();
});

test("waits for the first page without a client-side polling delay", async () => {
  vi.useFakeTimers();
  const batches: { wait_ms?: number }[] = [];
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) {
      batches.push(args.action.search_batch);
      if (batches.length === 1)
        return { kind: "search_page", data: { ...page, done: false } };
      return { kind: "search_page", data: page };
    }
    return { kind: "accepted" };
  });
  render(<Search partitions={[]} />);
  await tick(0);
  expect(batches).toHaveLength(2);
  expect(batches.every((batch) => batch.wait_ms === 1000)).toBe(true);
  expect(screen.getByText("movie.txt")).toBeTruthy();
});

test("search menu matching controls use one mode and apply only to the current query", async () => {
  vi.useFakeTimers();
  mockPage(page);
  const onCommands = vi.fn();
  render(
    <Search partitions={[]} initialText="movie" onCommands={onCommands} />,
  );
  await tick(0);
  const actions = () => onCommands.mock.lastCall?.[0].actions.search;
  act(() =>
    actions()
      .find((action: { shortcut?: string }) => action.shortcut === "Ctrl+B")
      .run(),
  );
  await tick(0);
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: {
      search_start: expect.objectContaining({
        options: expect.objectContaining({ mode: "whole_word" }),
      }),
    },
  });
  vi.mocked(invoke).mockClear();
  act(() =>
    actions()
      .find((action: { shortcut?: string }) => action.shortcut === "Ctrl+R")
      .run(),
  );
  await tick(0);
  expect(
    vi
      .mocked(invoke)
      .mock.calls.filter(([, args]: any) => args?.action?.search_start),
  ).toHaveLength(1);
  expect(
    actions().find(
      (action: { shortcut?: string }) => action.shortcut === "Ctrl+B",
    ).checked,
  ).toBe(false);
  expect(
    actions().find(
      (action: { shortcut?: string }) => action.shortcut === "Ctrl+R",
    ).checked,
  ).toBe(true);
});

test("waits for the IME to commit before searching", async () => {
  vi.useFakeTimers();
  mockPage(page);
  render(<Search partitions={[]} />);
  await tick(0);
  const input = screen.getByRole("searchbox");
  vi.mocked(invoke).mockClear();
  fireEvent.compositionStart(input);
  fireEvent.change(input, { target: { value: "dian" } });
  fireEvent.keyDown(input, { key: "Enter", isComposing: true });
  await tick(160);
  const starts = () =>
    vi
      .mocked(invoke)
      .mock.calls.filter(([, args]) =>
        Boolean(
          (args as { action: { search_start?: object } }).action.search_start,
        ),
      );
  expect(starts()).toHaveLength(0);
  fireEvent.change(input, { target: { value: "电影" } });
  fireEvent.compositionEnd(input, { data: "电影" });
  await tick(75);
  expect(starts()).toHaveLength(1);
  expect(starts()[0][1]).toMatchObject({
    action: { search_start: { text: "电影" } },
  });
});

function refreshFixture() {
  vi.useFakeTimers();
  let id = 0;
  const starts: string[] = [];
  const cancels: number[] = [];
  const batches: { id: number; offset: number; limit: number }[] = [];
  const fixture = {
    starts,
    cancels,
    batches,
    total: 1600,
    fail: false,
    delay: (..._batch: [id: number, offset: number]) => 0,
    row: (id: number, offset: number): SearchHit => ({
      ...hit,
      revision: id,
      file: { ...hit.file, name: `query-${id}-${offset}.txt` },
    }),
    reselect: (
      ..._query: [id: number, previous: number, offset: number]
    ): number | null => null,
  };
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) {
      starts.push(args.action.search_start.text);
      id++;
      if (fixture.fail) throw Error("索引暂时不可用");
      return { kind: "search_started", data: id };
    }
    if (args.action.search_cancel) {
      cancels.push(args.action.search_cancel.id);
      return { kind: "accepted" };
    }
    if (args.action.search_reselect) {
      const { id, previous, offset } = args.action.search_reselect;
      return {
        kind: "search_position",
        data: fixture.reselect(id, previous, offset),
      };
    }
    const batch = args.action.search_batch;
    batches.push(batch);
    const total = fixture.total;
    const count = Math.min(
      batch.limit,
      batch.id === 1 ? 32 : 4,
      total - batch.offset,
    );
    const delay = fixture.delay(batch.id, batch.offset);
    if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
    const rows = Array.from({ length: count }, (_, i) =>
      fixture.row(batch.id, batch.offset + i),
    );
    return {
      kind: "search_page",
      data: {
        ...page,
        id: batch.id,
        total,
        rows,
        next: batch.offset + count < total ? batch.offset + count : null,
      },
    };
  });
  return fixture;
}

test("preserves the selected entry and its preview snapshot when a new result moves it", async () => {
  const fixture = refreshFixture();
  const defaultRow = fixture.row;
  fixture.row = (id, offset) => {
    if (offset !== (id === 1 ? 1 : 3)) return defaultRow(id, offset);
    return {
      ...hit,
      revision: id,
      file: { ...hit.file, size: id === 1 ? "10" : "25" },
    };
  };
  fixture.reselect = (id, previous, offset) =>
    id === 2 && previous === 1 && offset === 1 ? 3 : null;
  const search = (revision: number) => (
    <Search
      partitions={[]}
      initialPreview
      indexes={{ p: publication(revision) }}
    />
  );
  const view = render(search(1));
  await tick(0);
  const list = screen.getByLabelText("搜索结果列表");
  const preview = screen.getByLabelText("预览窗格");
  fireEvent.click(within(list).getByText("movie.txt"));
  expect(within(preview).getByText("10 B")).toBeTruthy();

  view.rerender(search(2));
  await tick(250);
  const row = within(list).getByText("movie.txt").closest("tr")!;
  expect(row.getAttribute("data-index")).toBe("3");
  expect(row.getAttribute("aria-selected")).toBe("true");
  expect(within(preview).getByText("10 B")).toBeTruthy();
  fireEvent.click(row);
  expect(within(preview).getByText("25 B")).toBeTruthy();
});

test("defers applying an index publication while multiple results are selected", async () => {
  const fixture = refreshFixture();
  const search = (revision: number) => (
    <Search partitions={[]} indexes={{ p: publication(revision) }} />
  );
  const view = render(search(1));
  await tick(0);
  fireEvent.click(screen.getByText("query-1-0.txt"));
  fireEvent.click(screen.getByText("query-1-1.txt"), { ctrlKey: true });
  view.rerender(search(2));
  await tick(500);
  expect(screen.getByText("已选择 2 项")).toBeTruthy();
  expect(fixture.starts).toHaveLength(1);

  fireEvent.click(screen.getByText("query-1-0.txt"));
  await tick(250);
  expect(screen.getByText("query-2-0.txt")).toBeTruthy();
});

test("refreshes the preview on demand without clearing a surviving selection", async () => {
  const fixture = refreshFixture();
  const defaultRow = fixture.row;
  fixture.row = (id, offset) =>
    offset === 0
      ? { ...hit, revision: id, file: { ...hit.file, size: String(id * 10) } }
      : defaultRow(id, offset);
  fixture.reselect = () => 0;
  const onCommands = vi.fn();
  render(<Search partitions={[]} initialPreview onCommands={onCommands} />);
  await tick(0);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.click(within(list).getByText("movie.txt"));
  act(() => onCommands.mock.lastCall?.[0].refresh());
  await tick(250);
  expect(
    within(list)
      .getByText("movie.txt")
      .closest("tr")
      ?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(
    within(screen.getByLabelText("预览窗格")).getByText("20 B"),
  ).toBeTruthy();
});

test("defers and combines relevant index versions until the visible search stops scrolling", async () => {
  const { starts } = refreshFixture();
  const partitions = [{ id: "p", name: "Movies" }];
  const view = render(
    <Search
      partitions={partitions}
      indexes={{ p: publication(1), q: publication(1) }}
    />,
  );
  fireEvent.change(screen.getByRole("combobox", { name: "分区范围" }), {
    target: { value: "p" },
  });
  await tick(150);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search
      partitions={partitions}
      indexes={{ p: publication(1), q: publication(2) }}
    />,
  );
  await tick(400);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search
      partitions={partitions}
      active={false}
      indexes={{ p: publication(2) }}
    />,
  );
  await tick(400);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search partitions={partitions} indexes={{ p: publication(3) }} />,
  );
  const list = screen.getByLabelText("搜索结果列表");
  for (const scrollTop of [31000, 32000, 33000]) {
    fireEvent.scroll(list, { target: { scrollTop } });
    await tick(200);
    expect(starts).toHaveLength(1);
  }
  await tick(50);
  expect(starts).toHaveLength(2);
  expect(list.scrollTop).toBe(33000);
});

test("keeps the old viewport through short pages and ignores late old pagination", async () => {
  const fixture = refreshFixture();
  const partitions = [{ id: "p", name: "Movies" }];
  const view = render(
    <Search partitions={partitions} indexes={{ p: publication(1) }} />,
  );
  await tick(150);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 17000 } });
  await tick(0);
  expect(screen.getByText("query-1-492.txt")).toBeTruthy();
  fireEvent.click(screen.getByText("query-1-492.txt").closest("tr")!);
  fixture.delay = (id, offset) => (id === 2 && offset > 0 ? 30 : 0);
  view.rerender(
    <Search partitions={partitions} indexes={{ p: publication(2) }} />,
  );
  await tick(250);
  expect(screen.getByText("1,600 条结果")).toBeTruthy();
  expect(screen.getByText("query-1-492.txt")).toBeTruthy();
  expect(screen.queryByText("正在搜索…")).toBeNull();
  expect(list.scrollTop).toBe(17000);
  fireEvent.contextMenu(screen.getByText("query-1-492.txt").closest("tr")!);
  expect(screen.getByRole("menuitem", { name: "复制路径" })).toHaveProperty(
    "disabled",
    true,
  );
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  await tick(240);
  expect(
    fixture.batches.filter(({ id }) => id === 2).map(({ offset }) => offset),
  ).toEqual([0, 492, 496, 500, 504, 508, 512, 516, 520]);
  expect(screen.getByText("query-2-521.txt")).toBeTruthy();
  expect(screen.queryByText("query-1-492.txt")).toBeNull();
  expect(screen.queryByText("已选择 1 项")).toBeNull();
  expect(list.scrollTop).toBe(17000);
  fixture.delay = (id, offset) => (id === 2 && offset > 0 ? 600 : 0);
  fireEvent.scroll(list, { target: { scrollTop: 34000 } });
  view.rerender(
    <Search partitions={partitions} indexes={{ p: publication(3) }} />,
  );
  fixture.total = 50;
  await tick(250);
  expect(screen.getByText("50 条结果")).toBeTruthy();
  expect(list.scrollTop).toBe(1200);
  await tick(600);
  expect(screen.getByText("query-3-49.txt")).toBeTruthy();
  expect(screen.queryByText(/query-2-99/)).toBeNull();
});

test("keeps failed background results retryable and a new query supersedes the retry", async () => {
  const fixture = refreshFixture();
  const partitions = [{ id: "p", name: "Movies" }];
  const view = render(
    <Search partitions={partitions} indexes={{ p: publication(1) }} />,
  );
  await tick(150);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 17000 } });
  await tick(0);
  fixture.fail = true;
  view.rerender(
    <Search partitions={partitions} indexes={{ p: publication(2) }} />,
  );
  await tick(250);
  expect(screen.getByRole("alert").textContent).toContain("索引暂时不可用");
  expect(screen.getByText("query-1-492.txt")).toBeTruthy();
  expect(list.scrollTop).toBe(17000);
  fixture.fail = false;
  fixture.delay = (id, offset) => (id === 3 && offset > 0 ? 600 : 0);
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await tick(250);
  expect(fixture.starts).toHaveLength(3);
  expect(screen.getByText("query-1-492.txt")).toBeTruthy();
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "changed" },
  });
  expect(screen.getByText("正在搜索…")).toBeTruthy();
  await tick(150);
  expect(fixture.starts.at(-1)).toBe("changed");
  expect(fixture.cancels).toContain(1);
  expect(fixture.cancels).toContain(3);
  expect(list.scrollTop).toBe(0);
  await tick(600);
  expect(screen.getByText("query-4-0.txt")).toBeTruthy();
  expect(screen.queryByText(/query-3-/)).toBeNull();
});
