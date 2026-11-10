import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Desktop from "./Desktop";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
});
beforeEach(() => {
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get")
      return {
        version: 1,
        collapsed: false,
        start_page: "overview",
        include_path: true,
      };
    if (command === "preferences_set") return args.preferences;
    if (command === "asset_snapshot")
      return {
        version: 1,
        busy: false,
        error: null,
        assets: [],
        candidates: [],
      };
    if (command === "service_status") throw new Error("服务未启动");
    if (command === "app_version") return "0.1.0";
    if (args?.action?.search_start) return { kind: "search_started", data: 1 };
    if (args?.action?.search_batch)
      return {
        kind: "search_page",
        data: {
          id: 1,
          done: true,
          row: null,
          next: null,
          more: false,
          error: null,
          unavailable: [],
          rows: [],
          total: 0,
          groups: 0,
          missing: 0,
        },
      };
    return { kind: "accepted" };
  });
});
afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.resetAllMocks();
});

function menu(group: string, action: string) {
  fireEvent.click(screen.getByRole("button", { name: group }));
  fireEvent.click(screen.getByRole("menuitem", { name: action }));
}

test("starts in files and keeps the search input when switching to a singleton overview", async () => {
  render(<Desktop />);
  expect(screen.getByRole("tab", { name: "浏览" })).toHaveProperty(
    "ariaSelected",
    "true",
  );
  fireEvent.change(screen.getByRole("searchbox", { name: "搜索文件或路径" }), {
    target: { value: "aaa bbb" },
  });
  await waitFor(() =>
    expect(
      screen.getByRole("searchbox", { name: "搜索文件或路径" }),
    ).toHaveProperty("value", "aaa bbb"),
  );
  menu("查看", "概览（硬盘）");
  menu("查看", "概览（硬盘）");
  expect(screen.getAllByRole("tab", { name: "概览" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("tab", { name: "搜索" }));
  expect(
    screen.getByRole("searchbox", { name: "搜索文件或路径" }),
  ).toHaveProperty("value", "aaa bbb");
  expect(screen.getAllByRole("main")).toHaveLength(1);
});

test("keeps task activity out of the overview and reuses the tasks tab", async () => {
  render(<Desktop />);
  await screen.findByRole("button", { name: "登记分区…" });
  menu("查看", "概览（硬盘）");
  expect(screen.queryByRole("heading", { name: "任务动态" })).toBeNull();
  menu("工具", "任务");
  expect(screen.getByRole("button", { name: /最近动态/ })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /历史/ }));
  menu("查看", "概览（硬盘）");
  menu("工具", "任务");
  expect(screen.getAllByRole("tab", { name: "任务" })).toHaveLength(1);
  expect(screen.getByRole("button", { name: /历史/ })).toHaveProperty(
    "ariaPressed",
    "true",
  );
});

test("shows files immediately with legacy preferences and applies the search default", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args) =>
    command === "preferences_get"
      ? Promise.resolve({
          version: 1,
          collapsed: true,
          start_page: "settings",
          include_path: false,
        })
      : original(command, args),
  );
  render(<Desktop />);
  expect(screen.getByRole("tab", { name: "浏览" })).toBeTruthy();
  await screen.findByRole("button", { name: "登记分区…" });
  fireEvent.change(screen.getByRole("searchbox", { name: "搜索文件或路径" }), {
    target: { value: "test" },
  });
  fireEvent.click(within(screen.getByRole("tabpanel")).getByText("视图"));
  expect(screen.getByRole("checkbox", { name: "仅文件名" })).toHaveProperty(
    "checked",
    true,
  );
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_start: expect.objectContaining({
          text: "test",
          names_only: true,
          partitions: [],
        }),
      },
    }),
  );
});

test("keeps settings reachable when the service and preference storage fail", async () => {
  vi.mocked(invoke).mockRejectedValue("无法读取");
  render(<Desktop />);
  expect(screen.getByRole("tab", { name: "浏览" })).toBeTruthy();
  menu("工具", "设置");
  expect(await screen.findByRole("button", { name: "搜索" })).toBeTruthy();
  expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);
  expect(screen.queryByLabelText("启动页面")).toBeNull();
});

test("submitting the same overview query again refreshes results", async () => {
  render(<Desktop />);
  menu("查看", "概览（硬盘）");
  const search = screen.getByRole("searchbox", { name: "搜索所有文件" });
  fireEvent.change(search, { target: { value: "same query" } });
  fireEvent.submit(search.closest("form")!);
  await screen.findByText("没有匹配的文件");
  const first = vi
    .mocked(invoke)
    .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  fireEvent.click(screen.getByRole("tab", { name: "概览" }));
  fireEvent.submit(search.closest("form")!);
  await waitFor(() =>
    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter(([, args]: any) => args?.action?.search_start)
        .length,
    ).toBe(first + 1),
  );
});

test("opens registration over the active files tab and restores its trigger", async () => {
  render(<Desktop />);
  const entry = await screen.findByRole("button", { name: "登记分区…" });
  entry.focus();
  fireEvent.click(entry);
  expect(screen.getByRole("dialog", { name: "选择分区登记" })).toBeTruthy();
  expect(
    screen.getByRole("tab", { name: "浏览", selected: true }),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "关闭登记" }));
  await waitFor(() => expect(document.activeElement).toBe(entry));
  expect(
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "asset_snapshot"),
  ).toHaveLength(1);
});
