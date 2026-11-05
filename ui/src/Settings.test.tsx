import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeAll, expect, test, vi } from "vitest";
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
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});
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
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));
  fireEvent.change(screen.getByLabelText("启动页面"), {
    target: { value: "settings" },
  });
  await waitFor(() =>
    expect(screen.getByLabelText("启动页面")).toHaveProperty(
      "value",
      "settings",
    ),
  );
  view.unmount();
  render(<Desktop />);
  expect(
    (await screen.findByRole("button", { name: "设置" })).getAttribute(
      "aria-current",
    ),
  ).toBe("page");
  expect(screen.getByLabelText("启动页面")).toHaveProperty("value", "settings");
  fireEvent.click(screen.getByRole("button", { name: "搜索" }));
  fail = true;
  fireEvent.click(screen.getByLabelText("默认包含路径"));
  await screen.findByText(/磁盘写入失败/);
  expect(screen.getByLabelText("默认包含路径")).toHaveProperty("checked", true);
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
    if (args?.action?.task_page)
      return {
        kind: "tasks",
        data: {
          version: 1,
          row: null,
          next: null,
          automatic: enabled,
          paused: [],
        },
      };
    if (args?.action?.auto_update) {
      if (fail) throw new Error("策略写入失败");
      enabled = args.action.auto_update.enabled;
      return { kind: "accepted" };
    }
  });
  render(<Desktop />);
  fireEvent.click(await screen.findByRole("button", { name: "索引与同步" }));
  const toggle = await screen.findByRole("switch", {
    name: "自动更新已登记分区",
  });
  await waitFor(() => expect(toggle).toHaveProperty("disabled", false));
  fireEvent.click(toggle);
  expect(screen.getByRole("dialog")).toBeTruthy();
  expect(enabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "保持开启" }));
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
