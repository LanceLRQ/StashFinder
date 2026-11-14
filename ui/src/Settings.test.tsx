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
import Desktop from "./Desktop";
import Settings from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

function openSettings() {
  fireEvent.click(screen.getByRole("button", { name: "工具" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "设置…" }));
}

test.each(["portable", "windows_service"])(
  "offers an explicit portable exit only for the portable backend: %s",
  async (mode) => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "app_version") return "0.1.0";
      if (command === "service_status")
        return {
          version: 8,
          id: 1,
          process_id: 42,
          mode,
          registered_partitions: 0,
        };
    });
    render(
      <Settings
        preferences={{
          version: 1,
          collapsed: false,
          start_page: "overview",
          include_path: true,
          categories: [],
        }}
        saving={false}
        onSave={async () => {}}
        tasks={null}
        taskError=""
        onTasksRefresh={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "关于与诊断" }));
    await screen.findByText(mode);
    const exit = screen.queryByRole("button", { name: "退出便携版" });
    if (mode === "portable") {
      fireEvent.click(exit!);
      expect(invoke).toHaveBeenCalledWith("app_exit");
    } else expect(exit).toBeNull();
  },
);
test("saves preferences, restores them on restart, and retains saved values on failure", async () => {
  let saved = {
    version: 1,
    collapsed: false,
    start_page: "overview",
    include_path: true,
  };
  let fail = false;
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get") return saved;
    if (command === "preferences_set") {
      if (fail) throw new Error("磁盘写入失败");
      saved = args.preferences;
      return saved;
    }
    if (command === "app_version") return "0.1.0";
    if (command === "asset_snapshot")
      return {
        version: 1,
        busy: false,
        error: null,
        assets: [],
        candidates: [],
      };
    if (command === "service_status")
      return {
        version: 3,
        id: 1,
        process_id: 42,
        mode: "service",
        registered_partitions: 0,
      };
    return undefined;
  });
  const view = render(<Desktop />);
  openSettings();
  const search = within(screen.getByRole("group", { name: "搜索与筛选" }));
  const appearance = within(screen.getByRole("group", { name: "界面与布局" }));
  const include = search.getByRole("switch", { name: "默认包含路径" });
  expect(
    search.getByRole("combobox", { name: "浏览与新搜索默认筛选" }),
  ).toBeTruthy();
  expect(appearance.getByRole("switch", { name: "显示状态栏" })).toHaveProperty(
    "checked",
    true,
  );
  expect(
    appearance.getByRole("combobox", { name: "新标签默认布局" }),
  ).toHaveProperty("value", "details");
  await waitFor(() => expect(include).toHaveProperty("disabled", false));
  fireEvent.click(include);
  expect(saved.include_path).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull();
  openSettings();
  fireEvent.click(screen.getByLabelText("默认包含路径"));
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await waitFor(() => expect(include).toHaveProperty("checked", false));
  await waitFor(() => expect(saved.include_path).toBe(false));
  view.unmount();
  render(<Desktop />);
  expect(
    screen.getByRole("tab", { name: "概览", selected: true }),
  ).toBeTruthy();
  openSettings();
  await waitFor(() =>
    expect(screen.getByLabelText("默认包含路径")).toHaveProperty(
      "checked",
      false,
    ),
  );
  fail = true;
  fireEvent.click(screen.getByLabelText("默认包含路径"));
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await screen.findByText(/磁盘写入失败/);
  expect(screen.getByLabelText("默认包含路径")).toHaveProperty("checked", true);
  expect(saved.include_path).toBe(false);
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "设置" })).toBeNull(),
  );
  expect(saved.include_path).toBe(true);
});

test("shows actual version and service diagnostics, and can retry after a connection failure", async () => {
  let online = false;
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "preferences_get")
      return {
        version: 1,
        collapsed: false,
        start_page: "settings",
        include_path: true,
      };
    if (command === "app_version") return "0.1.0";
    if (command === "asset_snapshot")
      return {
        version: 1,
        busy: false,
        error: null,
        assets: [],
        candidates: [],
      };
    if (command === "service_status") {
      if (!online) throw new Error("管道未连接");
      return {
        version: 3,
        id: 1,
        process_id: 42,
        mode: "service",
        registered_partitions: 2,
      };
    }
  });
  render(<Desktop />);
  openSettings();
  fireEvent.click(await screen.findByRole("button", { name: "关于与诊断" }));
  expect(await screen.findByText("0.1.0")).toBeTruthy();
  expect(await screen.findByText(/管道未连接/)).toBeTruthy();
  expect(screen.queryByText("已连接")).toBeNull();
  online = true;
  fireEvent.click(screen.getByRole("button", { name: "重新连接" }));
  expect(await screen.findByText("已连接")).toBeTruthy();
  expect(screen.getByText("42")).toBeTruthy();
});

test("disabling automatic updates requires confirmation and a failed save retains the actual policy", async () => {
  let enabled = true;
  let fail = false;
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get")
      return {
        version: 1,
        collapsed: false,
        start_page: "settings",
        include_path: true,
      };
    if (command === "asset_snapshot")
      return {
        version: 1,
        busy: false,
        error: null,
        assets: [],
        candidates: [],
      };
    if (command === "app_version") return "0.1.0";
    if (command === "service_status")
      return {
        version: 4,
        id: 1,
        process_id: 42,
        mode: "service",
        registered_partitions: 0,
      };
    if (args?.action?.task_overview)
      return {
        kind: "task_overview",
        data: {
          version: "1",
          rows: [],
          next: null,
          automatic: enabled,
        },
      };
    if (args?.action?.auto_update) {
      if (fail) throw new Error("策略写入失败");
      enabled = args.action.auto_update.enabled;
      return { kind: "accepted" };
    }
  });
  render(<Desktop />);
  openSettings();
  fireEvent.click(await screen.findByRole("button", { name: "索引与同步" }));
  const toggle = await screen.findByRole("switch", {
    name: "自动更新已登记分区",
  });
  await waitFor(() => expect(toggle).toHaveProperty("disabled", false));
  fireEvent.click(toggle);
  expect(screen.getByRole("dialog", { name: "关闭自动更新？" })).toBeTruthy();
  expect(enabled).toBe(true);
  fireEvent.click(
    within(screen.getByRole("dialog", { name: "关闭自动更新？" })).getByRole(
      "button",
      { name: "关闭" },
    ),
  );
  expect(enabled).toBe(true);
  fireEvent.click(toggle);
  fail = true;
  fireEvent.click(screen.getByRole("button", { name: "关闭自动更新" }));
  await screen.findByText(/策略写入失败/);
  expect(toggle).toHaveProperty("checked", true);
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "关闭自动更新" }));
  await waitFor(() => expect(toggle).toHaveProperty("checked", false));
});
