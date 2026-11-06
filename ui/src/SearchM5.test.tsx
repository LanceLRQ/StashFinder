import {
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
import FileOperations from "./FileOperations";
import type { Asset } from "./asset-api";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("multi-selection preview uses shared facts only when the full selection is loaded", async () => {
  mock();
  render(<Search partitions={[]} />);
  fireEvent.click(await screen.findByText("file-0.txt"));
  fireEvent.click(screen.getByText("file-1.txt"), { shiftKey: true });
  fireEvent.click(screen.getByText("视图"));
  fireEvent.click(screen.getByLabelText("预览窗格"));
  const preview = within(
    screen.getByRole("complementary", { name: "预览窗格" }),
  );
  expect(preview.getByText("Movies")).toBeTruthy();
  expect(preview.getByText("离线")).toBeTruthy();
  fireEvent.keyDown(screen.getByLabelText("搜索结果列表"), {
    key: "a",
    ctrlKey: true,
  });
  expect(preview.getByText("已选择 10,000,000 项")).toBeTruthy();
  expect(preview.queryByText("Movies")).toBeNull();
  expect(preview.queryByText("离线")).toBeNull();
  expect(preview.queryByText("状态未确认")).toBeNull();
});
function mock(connection = "offline") {
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "file_prepare") throw Error("selection rejected");
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) {
      const { offset, limit } = args.action.search_batch;
      return {
        kind: "search_page",
        data: {
          id: 1,
          done: true,
          total: 10000000,
          groups: 3,
          missing: 2,
          next: offset + limit,
          rows: Array.from({ length: Math.min(limit, 20) }, (_, i) => ({
            partition_id: "p",
            filesystem_id: "fs",
            partition_name: "Movies",
            connection,
            mounts: connection === "online" ? ["E:\\"] : [],
            generation: 1,
            partial: false,
            indexed_at: null,
            needs_validation: false,
            hard_link: false,
            duplicate_group: 1,
            file: {
              entry: [i],
              parent: [0],
              object: [i],
              name: `file-${offset + i}.txt`,
              path: `folder/file-${offset + i}.txt`,
              kind: "file",
              size: "0",
              created: null,
              modified: null,
              attributes: 0,
            },
          })),
          unavailable: [],
          error: null,
        },
      };
    }
    return { kind: "accepted" };
  });
}
test("uses the same backend options for type, duplicates and match settings", async () => {
  mock();
  render(<Search partitions={[]} />);
  fireEvent.click(screen.getByText("筛选"));
  fireEvent.change(screen.getByLabelText("重复项"), {
    target: { value: "name;size" },
  });
  fireEvent.click(screen.getByLabelText("区分大小写"));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_start: expect.objectContaining({
          options: expect.objectContaining({
            case_sensitive: true,
            duplicate: ["name", "size"],
          }),
        }),
      },
    }),
  );
});

test("device changes refresh cached result status and menus without restarting the search", async () => {
  mock("online");
  const asset = {
    registration: {
      partition_id: "p",
      filesystem_id: "fs",
      name: "Movies",
      state: "ready",
    },
    connection: "online",
    mounts: ["E:\\"],
    error: null,
  } as Asset;
  const { rerender } = render(<Search partitions={[]} assets={[asset]} />);
  const item = await screen.findByText("file-0.txt");
  fireEvent.click(item);
  fireEvent.click(screen.getByText("视图"));
  fireEvent.click(screen.getByLabelText("预览窗格"));
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollLeft: 120 } });
  const starts = vi
    .mocked(invoke)
    .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  rerender(
    <Search
      partitions={[]}
      assets={[{ ...asset, connection: "offline", mounts: [] }]}
    />,
  );
  const row = within(list).getByText("file-0.txt").closest("tr")!;
  expect(row.getAttribute("aria-selected")).toBe("true");
  expect(within(row).getByText("离线")).toBeTruthy();
  expect(within(row).queryByText(/E:\\/)).toBeNull();
  expect(
    within(screen.getByRole("complementary", { name: "预览窗格" })).getByText(
      "离线",
    ),
  ).toBeTruthy();
  expect(list.scrollLeft).toBe(120);
  fireEvent.contextMenu(row, { clientX: 20, clientY: 30 });
  expect(
    screen.getByRole("menuitem", { name: /复制文件/ }).getAttribute("disabled"),
  ).not.toBeNull();
  rerender(
    <Search partitions={[]} assets={[{ ...asset, mounts: ["Z:\\"] }]} />,
  );
  expect(within(row).getByText("在线")).toBeTruthy();
  expect(within(row).getByText(/Z:\\/)).toBeTruthy();
  expect(
    screen.getByRole("menuitem", { name: /复制文件/ }).getAttribute("disabled"),
  ).toBeNull();
  rerender(
    <Search
      partitions={[]}
      assets={[
        {
          ...asset,
          registration: { ...asset.registration, filesystem_id: "replacement" },
          mounts: ["Z:\\"],
        },
      ]}
    />,
  );
  expect(within(row).getByText("历史记录")).toBeTruthy();
  expect(within(row).queryByText(/Z:\\/)).toBeNull();
  expect(
    vi
      .mocked(invoke)
      .mock.calls.filter(([, args]: any) => args?.action?.search_start),
  ).toHaveLength(starts);
});
test("keeps DOM rows bounded and requests distant result positions", async () => {
  mock();
  render(<Search partitions={[]} />);
  await screen.findByText("file-0.txt");
  expect(screen.getAllByRole("row").length).toBeLessThan(70);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 640000 } });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_batch: expect.objectContaining({ offset: expect.any(Number) }),
      },
    }),
  );
  await waitFor(
    () =>
      expect(
        screen.queryAllByText(/file-[1-9][0-9]{4,}.txt/).length,
      ).toBeGreaterThan(0),
    { timeout: 3000 },
  );
  expect(screen.getAllByRole("row").length).toBeLessThan(70);
  fireEvent.keyDown(list, { ctrlKey: true, key: "a" });
  expect(await screen.findByText("已选择 10,000,000 项")).toBeTruthy();
});
test("copies the full selection descriptor instead of the visible result cache", async () => {
  mock();
  render(
    <FileOperations>
      <Search partitions={[]} />
    </FileOperations>,
  );
  await screen.findByText("file-0.txt");
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.keyDown(list, { key: "a", ctrlKey: true });
  fireEvent.keyDown(list, { key: "c", ctrlKey: true });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("file_prepare", {
      input: {
        origin: { kind: "search", id: 1, range: [0, 9999999] },
        entries: [],
        positions: [],
      },
      recordsOnly: false,
    }),
  );
});
test("reports a failed cancellation request without an unhandled rejection", async () => {
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) return new Promise(() => {});
    if (args.action.search_cancel) throw Error("后台连接已断开");
    return { kind: "accepted" };
  });
  render(<Search partitions={[]} />);
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { search_batch: { id: 1, offset: 0, limit: 32 } },
    }),
  );
  fireEvent.click(screen.getByText("取消搜索"));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe(
      "取消请求失败：后台连接已断开",
    ),
  );
});
