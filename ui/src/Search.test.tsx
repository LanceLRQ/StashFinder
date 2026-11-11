import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Search from "./Search";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
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
  fireEvent.click(screen.getByText("筛选"));
  fireEvent.click(screen.getByRole("checkbox", { name: "仅文件名" }));
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

const task = { id: "reindex", partition_id: "p", updated: 1 };
const tick = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

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
    const rows = Array.from({ length: count }, (_, i) => ({
      ...hit,
      file: { ...hit.file, name: `query-${batch.id}-${batch.offset + i}.txt` },
    }));
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

test("defers and combines relevant task updates until the visible search stops scrolling", async () => {
  const { starts } = refreshFixture();
  const other = { id: "other", partition_id: "q", updated: 1 };
  const pruned = { id: "pruned", partition_id: "p", updated: 1 };
  const partitions = [{ id: "p", name: "Movies" }];
  const view = render(
    <Search partitions={partitions} completedTasks={[task, other, pruned]} />,
  );
  fireEvent.change(screen.getByRole("combobox", { name: "分区范围" }), {
    target: { value: "p" },
  });
  await tick(150);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search
      partitions={partitions}
      completedTasks={[{ ...other, updated: 2 }, task]}
    />,
  );
  await tick(400);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search
      partitions={partitions}
      active={false}
      completedTasks={[{ ...task, updated: 2 }]}
    />,
  );
  await tick(400);
  expect(starts).toHaveLength(1);
  view.rerender(
    <Search
      partitions={partitions}
      completedTasks={[{ ...task, updated: 3 }]}
    />,
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
    <Search partitions={partitions} completedTasks={[task]} />,
  );
  await tick(150);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 17000 } });
  await tick(0);
  expect(screen.getByText("query-1-492.txt")).toBeTruthy();
  fireEvent.click(screen.getByText("query-1-492.txt").closest("tr")!);
  fixture.delay = (id, offset) => (id === 2 && offset > 0 ? 30 : 0);
  view.rerender(
    <Search
      partitions={partitions}
      completedTasks={[{ ...task, updated: 2 }]}
    />,
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
    <Search
      partitions={partitions}
      completedTasks={[{ ...task, updated: 3 }]}
    />,
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
    <Search partitions={partitions} completedTasks={[task]} />,
  );
  await tick(150);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 17000 } });
  await tick(0);
  fixture.fail = true;
  view.rerender(
    <Search
      partitions={partitions}
      completedTasks={[{ ...task, updated: 2 }]}
    />,
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
