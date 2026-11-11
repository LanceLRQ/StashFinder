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

test.each([
  ["小", 104, 82, 6],
  ["中", 144, 108, 4],
  ["大", 208, 176, 3],
] as const)(
  "Ctrl frames disjoint %s search grid positions and passes exactly those positions to file operations",
  async (size, width, height, columns) => {
    mock("online", 1600);
    render(
      <FileOperations>
        <SearchWithView partitions={[]} />
      </FileOperations>,
    );
    const list = screen.getByLabelText("搜索结果列表");
    const results = within(list);
    await results.findByText("file-1.txt", {}, { timeout: 5000 });
    Object.defineProperties(list, {
      clientWidth: { value: 704 },
      clientHeight: { value: 420 },
    });
    vi.spyOn(list, "getBoundingClientRect").mockReturnValue(
      DOMRect.fromRect({ width: 704, height: 420 }),
    );
    fireEvent.resize(window);
    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    fireEvent.click(
      screen.getByRole("menuitemradio", { name: `缩略图（${size}图标）` }),
    );
    fireEvent.click(results.getByText("file-1.txt"));
    fireEvent.click(results.getByText("file-0.txt"), { ctrlKey: true });
    const pointer = {
      pointerId: 2,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
      ctrlKey: true,
    };
    fireEvent.pointerDown(list, { ...pointer, clientX: width, clientY: 5 });
    fireEvent.pointerMove(list, {
      ...pointer,
      clientX: 2 * width - 5,
      clientY: 2 * height + 6,
    });
    fireEvent.pointerUp(list, {
      ...pointer,
      clientX: 2 * width - 5,
      clientY: 2 * height + 6,
    });
    expect(screen.getByText("已选择 3 项")).toBeTruthy();
    fireEvent.keyDown(list, { key: "c", ctrlKey: true });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("file_prepare", {
        input: {
          origin: { kind: "search", id: 1, range: null },
          entries: [],
          positions: [0, columns + 1, 2 * columns + 1],
        },
        recordsOnly: false,
      }),
    );
  },
  15000,
);

test("frames an unloaded million-result window and blocks an oversized operation before IPC", async () => {
  const total = 1_000_000;
  mock("online", total);
  render(
    <FileOperations>
      <SearchWithView partitions={[]} />
    </FileOperations>,
  );
  const list = screen.getByLabelText("搜索结果列表");
  const view = within(list);
  await view.findByText("file-0.txt", {}, { timeout: 5000 });
  const height = 500;
  Object.defineProperties(list, {
    clientWidth: { value: 704 },
    clientHeight: { value: height },
  });
  vi.spyOn(list, "getBoundingClientRect").mockReturnValue(
    DOMRect.fromRect({ width: 704, height }),
  );
  fireEvent.resize(window);
  const pointer = { pointerId: 2, pointerType: "mouse", button: 0, buttons: 1 };
  fireEvent.pointerDown(list, { ...pointer, clientX: 1, clientY: 16 });
  fireEvent.pointerMove(list, { ...pointer, clientX: 240, clientY: 64 });
  const scale = (total * 64 - height) / (8_000_000 - height);
  fireEvent.scroll(list, { target: { scrollTop: (500_000 * 64) / scale } });
  fireEvent.pointerUp(list, { ...pointer, clientX: 240, clientY: 48 });
  expect(screen.getByText("已选择 500,001 项")).toBeTruthy();
  expect(screen.getByText(/文件操作最多支持 10,000 项/)).toBeTruthy();
  fireEvent.keyDown(list, { key: "c", ctrlKey: true });
  expect(
    vi
      .mocked(invoke)
      .mock.calls.some(([command]) => command === "file_prepare"),
  ).toBe(false);
  await view.findByText("file-500000.txt", {}, { timeout: 5000 });
  expect(
    view
      .getByText("file-500000.txt")
      .closest("tr")
      ?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(list.querySelectorAll("[data-index]").length).toBeLessThan(70);
}, 15000);

test.each([1600, 1_000_000])(
  "changes thumbnail sizes without restarting %i search results or losing the visible file",
  async (total) => {
    mock("offline", total);
    render(<SearchWithView partitions={[]} />);
    const list = screen.getByLabelText("搜索结果列表");
    const results = within(list);
    const waitForFile = (name: string) =>
      results.findByText(name, {}, { timeout: 5000 });
    await waitForFile("file-0.txt");
    Object.defineProperties(list, {
      clientWidth: { value: 704, configurable: true },
      clientHeight: { value: 500 },
    });
    fireEvent.resize(window);
    const middle = total / 2;
    const scale = (total * 64 - 500) / (Math.min(total * 64, 8_000_000) - 500);
    fireEvent.scroll(list, {
      target: { scrollTop: (middle * 64 + 7) / scale },
    });
    const name = `file-${middle}.txt`;
    fireEvent.click(await waitForFile(name));
    expect(screen.getByText("已选择 1 项")).toBeTruthy();
    const menu = within(screen.getByRole("navigation", { name: "程序菜单" }));
    const viewButton = menu.getByRole("button", { name: "查看" });
    const sizes = ["小", "中", "大"];
    const rowHeights = [82, 108, 176];
    for (const [index, size] of sizes.entries()) {
      fireEvent.click(viewButton);
      expect(
        menu
          .getByRole("menuitemradio", {
            name: index ? `缩略图（${sizes[index - 1]}图标）` : "详情",
          })
          .getAttribute("aria-checked"),
      ).toBe("true");
      fireEvent.click(
        menu.getByRole("menuitemradio", { name: `缩略图（${size}图标）` }),
      );
      const grid = results.getByRole("listbox");
      expect(list.style.getPropertyValue("--details-row-height")).toBe(
        `${rowHeights[index]}px`,
      );
      const item = (await waitForFile(name)).closest('[role="option"]')!;
      expect(item.getAttribute("aria-selected")).toBe("true");
      expect(item.getAttribute("aria-posinset")).toBe(String(middle + 1));
      expect(item.getAttribute("aria-setsize")).toBe(String(total));
      expect(grid.querySelectorAll('[role="option"]').length).toBeLessThan(100);
      expect(list.scrollTop).toBeGreaterThan(0);
    }
    // 704px fits three large tiles, 438px fits two. Keep the same result in view.
    const previousTop = list.scrollTop;
    Object.defineProperty(list, "clientWidth", {
      value: 438,
      configurable: true,
    });
    fireEvent.resize(window);
    const item = (await waitForFile(name)).closest('[role="option"]')!;
    expect(item.getAttribute("aria-selected")).toBe("true");
    const expectedHeight = Math.ceil(total / 2) * 176;
    const gridScale =
      (expectedHeight - 500) / (Math.min(expectedHeight, 8_000_000) - 500);
    const rowTop = Math.floor(middle / 2) * 176;
    expect(list.scrollTop).toBeCloseTo(rowTop / gridScale);
    if (total === 1600) expect(list.scrollTop).toBeGreaterThan(previousTop);
    fireEvent.keyDown(list, { key: "End" });
    await waitForFile(`file-${total - 1}.txt`);
    fireEvent.keyDown(list, { key: "Home" });
    fireEvent.click(await waitForFile("file-0.txt"));
    fireEvent.keyDown(document, { key: "v", altKey: true });
    fireEvent.click(menu.getByRole("menuitemradio", { name: "详情" }));
    const firstRow = (await waitForFile("file-0.txt")).closest("tr");
    expect(firstRow?.getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(firstRow);
    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter(
          ([command, args]: any[]) =>
            command === "index_request" && args.action.search_start,
        ),
    ).toHaveLength(1);
  },
  15000,
);

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
    expect(parseFloat(before.style.height)).toBe(
      Math.round(physicalTop - 8 * 64 - 17),
    );
    expect(
      parseFloat(before.style.height) +
        rendered() * 64 +
        parseFloat(after.style.height),
    ).toBe(content);
    fireEvent.scroll(list, { target: { scrollTop: content - height } });
    await screen.findByText(`file-${total - 1}.txt`);
    expect(table.getAttribute("aria-rowcount")).toBe(String(total + 1));
    expect(rendered()).toBeLessThan(70);
    expect(screen.queryByRole("button", { name: "加载更多文件" })).toBeNull();
  },
);
test("copies the full selection descriptor instead of the visible result cache", async () => {
  mock("online", 10_000);
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
        origin: { kind: "search", id: 1, range: [0, 9999] },
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
