import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Assets, { type AssetSnapshot } from "./Assets";

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

function snapshot(): AssetSnapshot {
  return {
    version: 1,
    busy: false,
    error: null,
    assets: [],
    candidates: ["A", "B"].map((id) => ({
      weak: false,
      blocked: null,
      observation: {
        candidate_id: id,
        disk_number: 1,
        model: "USB Test Disk",
        disk_bytes: 2000,
        partition_bytes: 1000,
        mounts: [id + ":\\"],
        identity: {
          layout: { kind: "gpt", disk: "disk", partition: id },
          filesystem: "NTFS",
          serial: { source: "ntfs64", value: id },
        },
        capacity: { total: 1000, free: 500, sampled_at: 1 },
        error: null,
      },
    })),
  };
}

test("starts unselected and registers only the selected partition", async () => {
  const data = snapshot();
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  const checkboxes = await screen.findAllByRole("checkbox");
  expect(checkboxes.every((c) => !(c as HTMLInputElement).checked)).toBe(true);
  fireEvent.click(checkboxes[0]);
  fireEvent.click(screen.getByRole("button", { name: "登记所选分区" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("asset_command", {
      action: {
        register: {
          version: 1,
          candidate_id: "A",
          name: "A:",
          allow_weak: false,
        },
      },
    }),
  );
  expect(
    vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === "asset_command"),
  ).toHaveLength(1);
});

test("shows offline capacity as a snapshot and renames only the software record", async () => {
  const data = snapshot();
  data.assets = [
    {
      registration: {
        disk_id: "disk",
        partition_id: "partition",
        filesystem_id: "fs",
        name: "Movies",
        revision: 1,
        state: "ready",
        observation: data.candidates[0].observation,
        last_seen: 1,
      },
      connection: "offline",
      mounts: [],
      error: null,
    },
  ];
  data.candidates = [];
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  expect(await screen.findByText("Movies")).toBeTruthy();
  expect(screen.getByText(/上次记录/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "重命名 Movies" }));
  fireEvent.change(screen.getByRole("textbox", { name: "分区名称" }), {
    target: { value: "电影" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存名称" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("asset_command", {
      action: { rename: { partition_id: "partition", name: "电影" } },
    }),
  );
});

test("shows service failure instead of an empty registration success state", async () => {
  vi.mocked(invoke).mockRejectedValue("后台服务不可用");
  render(<Assets />);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "后台服务不可用",
  );
  expect(screen.queryByText("尚未登记分区")).toBeNull();
});

test("a catalog error is not presented as a successful empty catalog", async () => {
  const data = snapshot();
  data.error = "资产库不可用，需要恢复";
  data.candidates = [];
  vi.mocked(invoke).mockResolvedValue(data);
  render(<Assets />);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    data.error,
  );
  expect(screen.queryByText("尚未登记分区")).toBeNull();
  expect(
    screen.queryByText("没有可显示的新设备。接入硬盘后刷新设备。"),
  ).toBeNull();
});

test("empty partition names cannot be submitted", async () => {
  const data = snapshot();
  vi.mocked(invoke).mockResolvedValue(data);
  render(<Assets />);
  fireEvent.click((await screen.findAllByRole("checkbox"))[0]);
  fireEvent.change(screen.getByRole("textbox", { name: "A: 自定义名称" }), {
    target: { value: "   " },
  });
  expect(screen.getByRole("button", { name: "登记所选分区" })).toHaveProperty(
    "disabled",
    true,
  );
});

test("late initial response cannot overwrite a newer snapshot", async () => {
  let resolveInitial!: (value: AssetSnapshot) => void;
  const old = snapshot();
  old.candidates.forEach((c) => {
    c.observation.model = "旧设备";
  });
  const fresh = snapshot();
  fresh.version = 2;
  fresh.candidates.forEach((c) => {
    c.observation.model = "新设备";
  });
  let first = true;
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command !== "asset_snapshot") return undefined;
    if (first) {
      first = false;
      return new Promise<AssetSnapshot>((resolve) => {
        resolveInitial = resolve;
      });
    }
    return fresh;
  });
  render(<Assets />);
  fireEvent.click(screen.getByRole("button", { name: "刷新设备" }));
  expect(await screen.findByText(/新设备/)).toBeTruthy();
  await act(async () => {
    resolveInitial(old);
  });
  expect(screen.queryByText(/旧设备/)).toBeNull();
});

test("partial registration keeps the failed selection and reports the failure", async () => {
  const data = snapshot();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "asset_snapshot") return structuredClone(data);
    const action = (args as { action: { register: { candidate_id: string } } })
      .action.register;
    if (action.candidate_id === "B") throw new Error("设备已断开");
    data.version++;
    data.candidates[0].blocked = "已登记";
    data.assets = [
      {
        registration: {
          disk_id: "disk",
          partition_id: "A",
          filesystem_id: "fs",
          name: "A:",
          revision: 1,
          state: "ready",
          observation: data.candidates[0].observation,
          last_seen: 1,
        },
        connection: "online",
        mounts: ["A:\\"],
        error: null,
      },
    ];
    return undefined;
  });
  render(<Assets />);
  const boxes = await screen.findAllByRole("checkbox");
  boxes.forEach((box) => fireEvent.click(box));
  fireEvent.click(screen.getByRole("button", { name: "登记所选分区" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    expect.stringContaining("设备已断开"),
  );
  expect(screen.getByText("已登记 1 个分区")).toBeTruthy();
  const after = screen.getAllByRole("checkbox");
  expect(after[0]).toHaveProperty("checked", false);
  expect(after[1]).toHaveProperty("checked", true);
});

test("weak identity requires explicit confirmation and cancel sends nothing", async () => {
  const data = snapshot();
  data.candidates[0].weak = true;
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  fireEvent.click((await screen.findAllByRole("checkbox"))[0]);
  fireEvent.click(screen.getByRole("button", { name: "登记所选分区" }));
  expect(screen.getByRole("dialog", { name: "身份信息不足" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(
    vi
      .mocked(invoke)
      .mock.calls.some(([command]) => command === "asset_command"),
  ).toBe(false);
});

test("escape dismisses the rename dialog without saving", async () => {
  const data = snapshot();
  data.assets = [
    {
      registration: {
        disk_id: "d",
        partition_id: "p",
        filesystem_id: "f",
        name: "A",
        revision: 1,
        state: "ready",
        observation: data.candidates[0].observation,
        last_seen: 1,
      },
      connection: "online",
      mounts: ["A:\\"],
      error: null,
    },
  ];
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  fireEvent.click(await screen.findByRole("button", { name: "重命名 A" }));
  fireEvent(
    screen.getByRole("dialog"),
    new Event("cancel", { bubbles: false, cancelable: true }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});
