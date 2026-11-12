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
import IndexActivity from "./IndexActivity";
import { readTasks, type Task, type TaskSnapshot } from "./task-api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

const running: Task = {
  id: "task-one",
  partition_id: "one",
  kind: "sync",
  mode: "journal",
  automatic: true,
  state: "running",
  phase: "replaying",
  created: 123,
  updated: 124,
  error: null,
};
const failed: Task = {
  ...running,
  id: "task-two",
  partition_id: "two",
  state: "failed",
  phase: null,
  error: "文件读取失败",
};
const completed: Task = {
  ...running,
  id: "task-three",
  kind: "rebuild",
  state: "completed",
  phase: null,
};
const data: TaskSnapshot = {
  tasks: [running, failed, completed],
  automatic: true,
  paused: ["two"],
  indexes: { two: { index_version: "1:1", indexed_at: 100 } },
};

function props() {
  return {
    error: "",
    data,
    partitions: [
      { id: "one", name: "Movies", canRetry: true, canChooseIndex: true },
      { id: "two", name: "Backup", canRetry: false, canChooseIndex: false },
    ],
    onSelect: vi.fn(),
    onRefresh: vi.fn(),
    onChooseIndex: vi.fn(),
  };
}

test("prioritizes issues and shows the selected partition's error and last successful update", () => {
  const activity = props();
  const view = render(<IndexActivity {...activity} />);
  const issue = screen.getByRole("heading", { name: "需要处理" });
  const work = screen.getByRole("heading", { name: "正在更新" });
  expect(
    issue.compareDocumentPosition(work) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Backup · 失败" }));
  expect(activity.onSelect).toHaveBeenCalledWith("two");
  view.rerender(<IndexActivity {...activity} partition="two" />);
  expect(screen.getByText("文件读取失败")).toBeTruthy();
  expect(screen.getByText(/最近成功更新/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "重试 Backup" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(screen.queryByRole("button", { name: "取消 Movies" })).toBeNull();
});

test("cancels by execution ID, resumes a partition and folds completed operations", async () => {
  vi.mocked(invoke).mockResolvedValue({ kind: "accepted" });
  const activity = props();
  render(<IndexActivity {...activity} />);
  expect(screen.getByText(/Movies · 补齐扫描期间变更/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "取消 Movies" }));
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: { task_cancel: { id: running.id } },
  });
  await waitFor(() => expect(activity.onRefresh).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "继续更新 Backup" }));
  expect(invoke).toHaveBeenCalledWith("index_request", {
    action: { partition_pause: { partition_id: "two", paused: false } },
  });
  await waitFor(() => expect(activity.onRefresh).toHaveBeenCalledTimes(2));
  const history = screen.getByText("最近操作").closest("details")!;
  expect(history.open).toBe(false);
  expect(within(history).getByText(/Movies · 完整校验 · 已完成/)).toBeTruthy();
});

test("retains the last activity and disables commands when the service disconnects", () => {
  render(<IndexActivity {...props()} error="服务未连接" />);
  expect(screen.getByRole("alert").textContent).toBe("服务未连接");
  expect(screen.getByRole("button", { name: "取消 Movies" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(
    screen.getByRole("button", { name: "继续更新 Backup" }),
  ).toHaveProperty("disabled", true);
});

test("restarts an expired overview page without mixing old publication versions", async () => {
  const partition = (index_version: string) => ({
    kind: "partition",
    data: { partition_id: "one", paused: true, indexed_at: 100, index_version },
  });
  vi.mocked(invoke)
    .mockResolvedValueOnce({
      kind: "task_overview",
      data: {
        version: "old",
        rows: [partition("1:1")],
        next: 1,
        automatic: true,
      },
    })
    .mockRejectedValueOnce("任务快照已更新，请重试")
    .mockResolvedValueOnce({
      kind: "task_overview",
      data: {
        version: "new",
        rows: [partition("1:2"), { kind: "task", data: running }],
        next: null,
        automatic: false,
      },
    });
  expect(await readTasks()).toEqual({
    tasks: [running],
    paused: ["one"],
    automatic: false,
    indexes: { one: { index_version: "1:2", indexed_at: 100 } },
  });
  expect(invoke).toHaveBeenNthCalledWith(2, "index_request", {
    action: { task_overview: { version: "old", offset: 1 } },
  });
});
