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
import { useState, type ComponentProps } from "react";
import Search from "./Search";
import AppMenu from "./AppMenu";
import FileOperations from "./FileOperations";
import type { FileCommandContext } from "./file-operations";
import type { Asset } from "./asset-api";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

function SearchWithView(props: ComponentProps<typeof Search>) {
  const [commands, setCommands] = useState<FileCommandContext | null>(null);
  return (
    <>
      <AppMenu
        groups={[
          { label: "查看", accessKey: "v", items: commands?.views ?? [] },
        ]}
      />
      <Search {...props} onCommands={setCommands} />
    </>
  );
}

function openPreview() {
  fireEvent.click(screen.getByRole("button", { name: "查看" }));
  fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "预览窗格" }));
}

test("multi-selection preview uses shared facts only when the full selection is loaded", async () => {
  mock();
  render(<SearchWithView partitions={[]} />);
  fireEvent.click(await screen.findByText("file-0.txt"));
  fireEvent.click(screen.getByText("file-1.txt"), { shiftKey: true });
  openPreview();
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
function mock(connection = "offline", total = 10_000_000) {
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "file_prepare") throw Error("selection rejected");
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) {
      const { offset, limit } = args.action.search_batch;
      const rowCount = Math.min(limit, 20, total - offset);
      return {
        kind: "search_page",
        data: {
          id: 1,
          done: true,
          total,
          groups: 3,
          missing: 2,
          next: offset + rowCount < total ? offset + rowCount : null,
          rows: Array.from({ length: rowCount }, (_, i) => ({
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
  const { rerender } = render(
    <SearchWithView partitions={[]} assets={[asset]} />,
  );
  const item = await screen.findByText("file-0.txt");
  fireEvent.click(item);
  openPreview();
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollLeft: 120 } });
  const starts = vi
    .mocked(invoke)
    .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  rerender(
    <SearchWithView
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
    <SearchWithView
      partitions={[]}
      assets={[{ ...asset, mounts: ["Z:\\"] }]}
    />,
  );
  expect(within(row).getByText("在线")).toBeTruthy();
  expect(within(row).getByText(/Z:\\/)).toBeTruthy();
  expect(
    screen.getByRole("menuitem", { name: /复制文件/ }).getAttribute("disabled"),
  ).toBeNull();
  rerender(
    <SearchWithView
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
test.each([1600, 1_000_000])(
  "keeps %i search results continuous across fractional rows and the end",
  async (total) => {
    mock("offline", total);
    render(<Search partitions={[]} />);
    await screen.findByText("file-0.txt");
    const list = screen.getByLabelText("搜索结果列表");
    const height = 500;
    const content = Math.min(total * 64, 8_000_000);
    const scale = (total * 64 - height) / (content - height);
    const middle = total / 2;
    const logicalTop = middle * 64 + 17;
    const physicalTop = logicalTop / scale;
    Object.defineProperty(list, "clientHeight", { value: height });
    fireEvent.scroll(list, { target: { scrollTop: physicalTop } });
    await screen.findByText(`file-${middle}.txt`);
    const table = within(list).getByRole("table");
    const spacers = () =>
      Array.from(
        table.querySelectorAll<HTMLTableRowElement>(
          'tbody tr[aria-hidden="true"]',
        ),
      );
    const rendered = () =>
      table.querySelectorAll("tbody tr[aria-rowindex]").length;
    const [before, after] = spacers();
    expect(parseFloat(before.style.height)).toBeCloseTo(
      physicalTop - 8 * 64 - 17,
    );
    expect(
      parseFloat(before.style.height) +
        rendered() * 64 +
        parseFloat(after.style.height),
    ).toBeCloseTo(content);
    fireEvent.scroll(list, { target: { scrollTop: content - height } });
    await screen.findByText(`file-${total - 1}.txt`);
    expect(table.getAttribute("aria-rowcount")).toBe(String(total + 1));
    expect(rendered()).toBeLessThan(70);
    expect(screen.queryByRole("button", { name: "加载更多文件" })).toBeNull();
  },
);
test("copies the full selection descriptor instead of the visible result cache", async () => {
  mock("online");
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
