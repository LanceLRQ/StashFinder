import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
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

async function openRegistration() {
  const button = screen.getByRole("button", { name: "登记分区" });
  await waitFor(() => expect(button).toHaveProperty("disabled", false));
  fireEvent.click(button);
}

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

test("registration is a dialog and closing it preserves the overview filter", async () => {
  vi.mocked(invoke).mockResolvedValue(snapshot());
  render(<Assets />);
  expect(screen.queryByRole("checkbox")).toBeNull();
  fireEvent.change(screen.getByLabelText("连接筛选"), {
    target: { value: "offline" },
  });
  await openRegistration();
  const dialog = await screen.findByRole("dialog", { name: "选择分区登记" });
  expect(within(dialog).getAllByRole("checkbox")).toHaveLength(2);
  const actions = dialog.querySelector<HTMLElement>(".dialog-actions");
  expect(actions).toBeTruthy();
  expect(
    within(actions!)
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual(["关闭登记", "登记所选分区"]);
  fireEvent.click(within(dialog).getByRole("button", { name: "关闭登记" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByLabelText("连接筛选")).toHaveProperty("value", "offline");
});

test("device processing updates the reserved overview footer", async () => {
  vi.mocked(invoke).mockResolvedValue({ ...snapshot(), busy: true });
  const { container } = render(<Assets />);
  const footer = container.querySelector("footer")!;
  const status = within(footer).getByRole("status");
  expect(await screen.findByText("正在处理设备信息…")).toBe(status);
});

test("groups partitions by disk identity and keeps the last snapshot on service failure", async () => {
  const data = snapshot();
  data.assets = data.candidates.map((c, i) => ({
    registration: {
      disk_id: "same-disk",
      partition_id: c.observation.candidate_id,
      filesystem_id: "fs" + i,
      name: i ? "Backup" : "Movies",
      revision: 1,
      state: "ready",
      observation: {
        ...c.observation,
        capacity: i ? null : c.observation.capacity,
      },
      last_seen: 1,
    },
    connection: i ? "uncertain" : "online",
    mounts: i ? [] : ["A:\\"],
    error: null,
  }));
  data.candidates = [];
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  await screen.findByText("Movies");
  expect(
    screen.getAllByRole("heading", { name: /USB Test Disk/ }),
  ).toHaveLength(1);
  expect(screen.getByText("身份待确认")).toBeTruthy();
  expect(
    within(screen.getByText("Backup").closest("tr")!).getByText("未知 / 未知"),
  ).toBeTruthy();
  vi.mocked(invoke).mockRejectedValue("服务连接中断");
  fireEvent.click(screen.getByRole("button", { name: "刷新设备" }));
  await screen.findAllByText("服务连接中断");
  expect(screen.getByText("Movies")).toBeTruthy();
  expect(
    within(screen.getByText("Movies").closest("tr")!).getByText("在线"),
  ).toBeTruthy();
  expect(screen.queryByText("尚未登记分区")).toBeNull();
});

test("starts unselected and registers only the selected partition", async () => {
  const data = snapshot();
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot" ? data : undefined,
  );
  render(<Assets />);
  await openRegistration();
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
  await openRegistration();
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
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "登记分区" })).toHaveProperty(
      "disabled",
      false,
    ),
  );
  fireEvent.click(screen.getByRole("button", { name: "登记分区" }));
  expect(await screen.findByRole("heading", { name: /新设备/ })).toBeTruthy();
  await act(async () => {
    resolveInitial(old);
  });
  expect(screen.queryByText(/旧设备/)).toBeNull();
});

test("partial registration keeps the failed selection and reports the failure", async () => {
  const data = snapshot();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "asset_snapshot") return structuredClone(data);
    if (command === "index_request")
      return {
        kind: "index",
        data: {
          generation: null,
          active_generation: null,
          phase: "not_indexed",
          started: null,
          completed: null,
          records: 0,
          error: null,
        },
      };
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
  await openRegistration();
  const boxes = await screen.findAllByRole("checkbox");
  boxes.forEach((box) => fireEvent.click(box));
  fireEvent.click(screen.getByRole("button", { name: "登记所选分区" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    expect.stringContaining("设备已断开"),
  );
  expect(screen.getByLabelText("已登记 1 个分区")).toBeTruthy();
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
  await openRegistration();
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

test("capacity failure shows historical values and used space without disabling scanning", async () => {
  const data = snapshot();
  const o = data.candidates[0].observation;
  o.identity.filesystem = "exFAT";
  data.assets = [
    {
      registration: {
        disk_id: "d",
        partition_id: "fat",
        filesystem_id: "f",
        name: "Archive",
        revision: 1,
        state: "ready",
        observation: o,
        last_seen: 2,
      },
      connection: "online",
      mounts: ["X:\\"],
      error: null,
      capacity_error: "容量读取失败，保留上次成功快照",
    },
  ];
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "asset_snapshot"
      ? data
      : { kind: "index", data: { phase: "not_indexed", completed: null } },
  );
  render(<Assets />);
  await screen.findByText("容量读取失败，保留上次成功快照");
  expect(screen.getByText("已用 500 B")).toBeTruthy();
  expect(screen.getByText(/上次记录/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "索引 Archive" })).toHaveProperty(
    "disabled",
    false,
  );
});
