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
};
function mockPage(value: object) {
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_page) return { kind: "search_page", data: value };
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

test("shows offline relative paths, unknown metadata and bounded results without file operations", async () => {
  mockPage(page);
  render(<Search partitions={[]} onBack={() => {}} />);
  expect(await screen.findByText("movie.txt")).toBeTruthy();
  expect(screen.getByText("aaa\\movie.txt")).toBeTruthy();
  expect(screen.getByText("离线")).toBeTruthy();
  expect(screen.getAllByText("未知").length).toBeGreaterThan(0);
  expect(screen.getByText(/请缩小搜索范围/)).toBeTruthy();
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
        search_start: { text: "aaa bbb", names_only: true, partitions: ["p"] },
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
    if (args.action.search_page?.id === 1)
      return new Promise((r) => {
        resolveOld = r;
      });
    if (args.action.search_page)
      return {
        kind: "search_page",
        data: {
          ...page,
          id: 2,
          row: {
            ...hit,
            partial: true,
            file: { ...hit.file, name: "new.txt" },
          },
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
