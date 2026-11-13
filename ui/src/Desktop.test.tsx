import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Desktop from "./Desktop";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
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
    if (args?.action?.task_overview)
      return {
        kind: "task_overview",
        data: {
          version: "1",
          rows: [],
          next: null,
          automatic: true,
        },
      };
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

test("remembers the browse preset and copies it only into new search tabs", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  let saved: any = await original("preferences_get");
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get") return saved;
    if (command === "preferences_set") return (saved = args.preferences);
    return original(command, args);
  });
  let desktop = render(<Desktop />);
  menu("文件", "新建浏览标签页");
  menu("搜索", "筛选预设");
  fireEvent.click(await screen.findByRole("menuitemradio", { name: "图片" }));
  await waitFor(() => expect(saved.browse_filter).toBe("category:图片"));
  desktop.unmount();
  desktop = render(<Desktop />);
  menu("文件", "新建浏览标签页");
  menu("搜索", "筛选预设");
  await waitFor(() =>
    expect(
      screen.getByRole("menuitemradio", {
        name: "图片",
        checked: true,
      }),
    ).toBeTruthy(),
  );
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  menu("文件", "新建搜索标签页");
  const starts = () =>
    vi
      .mocked(invoke)
      .mock.calls.flatMap(([, args]: any) =>
        args?.action?.search_start ? [args.action.search_start] : [],
      );
  await waitFor(() =>
    expect(starts().at(-1)?.options.extensions).toContain("jpg"),
  );
  const first = screen.getByRole("searchbox", { name: "搜索文件或路径" });
  fireEvent.change(first, { target: { value: "first" } });
  fireEvent.click(screen.getByRole("button", { name: /筛选器/ }));
  let dialog = screen.getByRole("dialog", { name: "筛选器" });
  fireEvent.click(within(dialog).getByRole("button", { name: "清除筛选" }));
  fireEvent.click(within(dialog).getByRole("button", { name: "应用" }));
  await waitFor(() =>
    expect(
      starts().find((r) => r.text === "first")?.options.extensions,
    ).toEqual([]),
  );
  menu("文件", "新建搜索标签页");
  await waitFor(() =>
    expect(starts().at(-1)?.options.extensions).toContain("jpg"),
  );
  fireEvent.click(screen.getByRole("tab", { name: "搜索：first" }));
  fireEvent.click(screen.getByRole("button", { name: /筛选器/ }));
  dialog = screen.getByRole("dialog", { name: "筛选器" });
  expect(within(dialog).getByRole("checkbox", { name: "图片" })).toHaveProperty(
    "checked",
    false,
  );
  expect(saved.browse_filter).toBe("category:图片");
});

test("saves only reusable conditions and does not apply a draft closed with X", async () => {
  render(<Desktop />);
  menu("文件", "新建搜索标签页");
  await screen.findByText("没有匹配的文件");
  fireEvent.click(screen.getByRole("button", { name: "筛选器" }));
  const dialog = within(screen.getByRole("dialog", { name: "筛选器" }));
  fireEvent.change(dialog.getByLabelText("自定义扩展名"), {
    target: { value: "pdf" },
  });
  fireEvent.change(dialog.getByLabelText("最小字节数"), {
    target: { value: "100" },
  });
  fireEvent.click(dialog.getByRole("checkbox", { name: "仅在线" }));
  fireEvent.change(dialog.getByLabelText("预设名称"), {
    target: { value: "大型 PDF" },
  });
  fireEvent.click(dialog.getByRole("button", { name: "另存为预设" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("preferences_set", {
      preferences: expect.objectContaining({
        filter_presets: [
          expect.objectContaining({
            name: "大型 PDF",
            rule: expect.objectContaining({
              extensions: ["pdf"],
              size: { min: "100", max: null },
            }),
          }),
        ],
      }),
    }),
  );
  const saved = vi
    .mocked(invoke)
    .mock.calls.find(([command]) => command === "preferences_set")![1] as any;
  expect(saved.preferences.filter_presets[0].rule).not.toHaveProperty(
    "online_only",
  );
  fireEvent.click(dialog.getByRole("button", { name: "关闭" }));
  fireEvent.click(screen.getByRole("button", { name: "筛选器" }));
  const reopened = within(screen.getByRole("dialog", { name: "筛选器" }));
  expect(reopened.getByLabelText("自定义扩展名")).toHaveProperty("value", "");
  expect(reopened.getByRole("checkbox", { name: "仅在线" })).toHaveProperty(
    "checked",
    false,
  );
  expect(reopened.getByRole("option", { name: "大型 PDF" })).toBeTruthy();
});

test("refreshes the active search from the menu and F5 without intercepting IME", async () => {
  render(<Desktop />);
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "film" },
  });
  fireEvent.submit(screen.getByRole("searchbox").closest("form")!);
  const input = await screen.findByRole("searchbox", {
    name: "搜索文件或路径",
  });
  await screen.findByText("没有匹配的文件");
  const starts = () =>
    vi
      .mocked(invoke)
      .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  const initial = starts();
  input.focus();
  fireEvent.keyDown(input, { altKey: true, key: "v" });
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  expect(document.activeElement).toBe(input);
  menu("查看", "刷新当前页");
  await waitFor(() => expect(starts()).toBe(initial + 1));
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "取消搜索" })).toBeNull(),
  );
  fireEvent.keyDown(input, { key: "F5", isComposing: true });
  expect(screen.queryByRole("button", { name: "取消搜索" })).toBeNull();
  fireEvent.keyDown(input, { key: "F5" });
  expect(screen.queryByRole("button", { name: "取消搜索" })).toBeNull();
  expect(screen.getByText("正在搜索…")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "查看" }));
  expect(screen.getByRole("menuitem", { name: "刷新当前页" })).toHaveProperty(
    "ariaDisabled",
    "true",
  );
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  await waitFor(() => expect(starts()).toBe(initial + 2));
  expect(screen.queryByRole("button", { name: "重新搜索" })).toBeNull();
  expect(screen.queryByText("视图")).toBeNull();
}, 12000);

test("refreshes the overview without overlapping a device refresh", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  let finishRefresh: (() => void) | undefined;
  vi.mocked(invoke).mockImplementation((command, args: any) => {
    if (
      command === "asset_command" &&
      args.action === "refresh" &&
      !finishRefresh
    )
      return new Promise<void>((resolve) => {
        finishRefresh = resolve;
      });
    return original(command, args);
  });
  render(<Desktop />);
  await screen.findByRole("button", { name: "登记分区…" });
  const deviceRefreshes = () =>
    vi
      .mocked(invoke)
      .mock.calls.filter(
        ([cmd, args]: any) =>
          cmd === "asset_command" && args.action === "refresh",
      ).length;
  fireEvent.click(screen.getByRole("tab", { name: "概览" }));
  const overview = screen.getByRole("tabpanel", { name: "概览" });
  menu("查看", "刷新当前页");
  expect(deviceRefreshes()).toBe(1);
  expect(within(overview).getByText("正在处理设备信息…")).toBeTruthy();
  fireEvent.keyDown(overview, { key: "F5" });
  expect(deviceRefreshes()).toBe(1);
  await act(async () => finishRefresh!());
  await waitFor(() =>
    expect(
      within(overview).getByRole("button", { name: "登记分区…" }),
    ).toHaveProperty("disabled", false),
  );
  fireEvent.keyDown(overview, { key: "F5" });
  expect(deviceRefreshes()).toBe(2);
  expect(
    within(overview).queryByRole("button", { name: "刷新设备" }),
  ).toBeNull();
  expect(
    within(overview).queryByRole("button", { name: "登记分区" }),
  ).toBeNull();
  expect(
    await within(overview).findByRole("button", { name: "登记分区…" }),
  ).toBeTruthy();
});

test("starts in Home and keeps the query when switching back to search", async () => {
  render(<Desktop />);
  expect(screen.getByRole("tab", { name: "概览" })).toHaveProperty(
    "ariaSelected",
    "true",
  );
  const overviewSearch = screen.getByRole("searchbox", {
    name: "搜索所有文件",
  });
  fireEvent.change(overviewSearch, { target: { value: "aaa bbb" } });
  fireEvent.submit(overviewSearch.closest("form")!);
  await waitFor(() =>
    expect(
      screen.getByRole("searchbox", { name: "搜索文件或路径" }),
    ).toHaveProperty("value", "aaa bbb"),
  );
  fireEvent.click(screen.getByRole("tab", { name: "概览" }));
  fireEvent.click(screen.getByRole("tab", { name: "概览" }));
  expect(screen.getAllByRole("tab", { name: "概览" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("tab", { name: "搜索：aaa bbb" }));
  expect(
    screen.getByRole("searchbox", { name: "搜索文件或路径" }),
  ).toHaveProperty("value", "aaa bbb");
  expect(screen.getAllByRole("main")).toHaveLength(1);
});

test("shows a running build ahead of an older failure and resolves other indexing issues", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  const assets = ["Movies", "Backup"].map((name) => ({
    registration: {
      disk_id: name,
      partition_id: name,
      filesystem_id: name,
      name,
      state: "ready",
      observation: { identity: { filesystem: "NTFS" }, mounts: [] },
    },
    connection: "online",
    mounts: [],
  }));
  let running = true;
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "asset_snapshot")
      return { version: 1, busy: false, error: null, assets, candidates: [] };
    if (args?.action?.task_overview) {
      const rows = [
        ...(running
          ? [
              {
                id: "running-task",
                partition_id: "Movies",
                state: "running",
                error: null,
              },
            ]
          : []),
        {
          id: "failed-task",
          partition_id: "Movies",
          state: "failed",
          error: "读取失败",
        },
        {
          id: "journal-task",
          partition_id: "Backup",
          state: "needs_confirmation",
          error: "此分区没有 USN 日志",
        },
      ].map((task) => ({
        kind: "task",
        data: {
          ...task,
          kind: "build",
          mode: "journal",
          automatic: true,
          created: 1,
          updated: 1,
        },
      }));
      return {
        kind: "task_overview",
        data: { version: "1", rows, next: null, automatic: true },
      };
    }
    return original(command, args);
  });
  render(<Desktop />);
  const sidebar = within(
    screen.getByRole("complementary", { name: "分区详情" }),
  );
  const card = within(await screen.findByRole("button", { name: "Movies" }));
  await card.findByText("索引中");
  expect(card.queryByText("待处理")).toBeNull();
  expect(sidebar.queryByRole("button", { name: "重试 Movies" })).toBeNull();
  running = false;
  await waitFor(() => expect(card.getByText("待处理")).toBeTruthy(), {
    timeout: 3500,
  });
  fireEvent.click(await sidebar.findByRole("button", { name: "重试 Movies" }));
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: { task_retry: { id: "failed-task" } },
  });
  const choose = sidebar.getByRole("button", { name: "选择索引方式 Backup" });
  await waitFor(() => expect(choose).toHaveProperty("disabled", false));
  fireEvent.click(choose);
  const dialog = screen.getByRole("dialog", { name: "选择索引方式 Backup" });
  expect(
    within(dialog).getByRole("radio", { name: /基础扫描/ }),
  ).toHaveProperty("checked", true);
  fireEvent.click(within(dialog).getByRole("button", { name: "开始索引" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { index_start: { partition_id: "Backup", mode: "scan_only" } },
    }),
  );
  expect(screen.getByRole("tab", { name: "概览" })).toHaveProperty(
    "ariaSelected",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "工具" }));
  expect(screen.queryByRole("menuitem", { name: "任务" })).toBeNull();
});

test("shows Home with legacy preferences and applies the search default", async () => {
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
  expect(screen.getByRole("tab", { name: "概览" })).toBeTruthy();
  await screen.findByRole("button", { name: "登记分区…" });
  const overviewSearch = screen.getByRole("searchbox", {
    name: "搜索所有文件",
  });
  fireEvent.change(overviewSearch, { target: { value: "test" } });
  fireEvent.submit(overviewSearch.closest("form")!);
  await screen.findByRole("searchbox", { name: "搜索文件或路径" });
  fireEvent.click(
    within(screen.getByRole("tabpanel")).getByRole("button", {
      name: "筛选器",
    }),
  );
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
  expect(screen.getByRole("tab", { name: "概览" })).toBeTruthy();
  menu("工具", "设置…");
  const dialog = await screen.findByRole("dialog", { name: "设置" });
  expect(within(dialog).getByRole("button", { name: "常用" })).toBeTruthy();
  expect(within(dialog).getByLabelText("默认包含路径")).toHaveProperty(
    "disabled",
    true,
  );
  expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);
  expect(screen.queryByLabelText("启动页面")).toBeNull();
});

test("Ctrl+P restores search focus without restarting its query", async () => {
  render(<Desktop />);
  menu("文件", "新建搜索标签页");
  const input = await screen.findByRole("searchbox", {
    name: "搜索文件或路径",
  });
  await screen.findByText("没有匹配的文件");
  const starts = () =>
    vi
      .mocked(invoke)
      .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  const before = starts();
  input.focus();
  fireEvent.keyDown(input, { key: "p", ctrlKey: true });
  const settings = screen.getByRole("dialog", { name: "设置" });
  expect(document.activeElement).toBe(
    within(settings).getByRole("button", { name: "常用" }),
  );
  fireEvent.keyDown(settings, { key: "f", ctrlKey: true, shiftKey: true });
  fireEvent(settings, new Event("cancel", { bubbles: true, cancelable: true }));
  expect(document.activeElement).toBe(input);
  expect(starts()).toBe(before);
  expect(screen.getAllByRole("tab")).toHaveLength(2);
});

test("manages personal presets in settings and applies them to new searches", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  let saved: any = await original("preferences_get");
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get") return saved;
    if (command === "preferences_set") return (saved = args.preferences);
    return original(command, args);
  });
  render(<Desktop />);
  menu("搜索", "管理筛选预设…");
  const settings = screen.getByRole("dialog", { name: "设置" });
  const controls = within(settings);
  const name = controls.getByLabelText("预设名称");
  await waitFor(() =>
    expect(controls.getByRole("button", { name: "常用" })).toBeTruthy(),
  );
  fireEvent.change(name, { target: { value: "大图片" } });
  fireEvent.click(controls.getByRole("checkbox", { name: "图片" }));
  await waitFor(() =>
    expect(controls.getByRole("button", { name: "另存为预设" })).toHaveProperty(
      "disabled",
      false,
    ),
  );
  fireEvent.click(controls.getByRole("button", { name: "另存为预设" }));
  expect(saved.filter_presets).toBeUndefined();
  fireEvent.click(controls.getByRole("button", { name: "常用" }));
  const filter = controls.getByRole("combobox", {
    name: "浏览与新搜索默认筛选",
  });
  const entry = controls.getByRole("option", {
    name: "个人：大图片",
  }) as HTMLOptionElement;
  fireEvent.change(filter, { target: { value: entry.value } });
  fireEvent.click(controls.getByRole("button", { name: "确定" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull(),
  );
  expect(saved.browse_filter).toBe(entry.value);
  menu("文件", "新建搜索标签页");
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_start: expect.objectContaining({
          options: expect.objectContaining({
            extensions: expect.arrayContaining(["jpg"]),
          }),
        }),
      },
    }),
  );
});

test("submitting the same overview query again refreshes results", async () => {
  render(<Desktop />);
  fireEvent.click(screen.getByRole("tab", { name: "概览" }));
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

test("opens registration over Home and restores its trigger", async () => {
  render(<Desktop />);
  const entry = await screen.findByRole("button", { name: "登记分区…" });
  entry.focus();
  fireEvent.click(entry);
  expect(screen.getByRole("dialog", { name: "选择分区登记" })).toBeTruthy();
  expect(
    screen.getByRole("tab", { name: "概览", selected: true }),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "关闭" }));
  await waitFor(() => expect(document.activeElement).toBe(entry));
  expect(
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "asset_snapshot"),
  ).toHaveLength(1);
});
