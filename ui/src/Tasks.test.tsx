import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Tasks from "./Tasks";
import type { Task } from "./task-api";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});
const row: Task = {
  id: "task-one",
  partition_id: "one",
  kind: "sync",
  mode: "journal",
  automatic: true,
  state: "running",
  created: 123,
  updated: 124,
  error: null,
};
const partitions = [
  { id: "one", name: "Movies", connection: "online" },
  { id: "two", name: "Backup", connection: "offline" },
];
test("cancels the selected task by id and disables offline retries", async () => {
  vi.mocked(invoke).mockResolvedValue({ kind: "accepted" });
  render(
    <Tasks
      data={{
        tasks: [
          row,
          {
            ...row,
            id: "task-two",
            partition_id: "two",
            state: "failed",
            error: "断开",
          },
        ],
        automatic: true,
        paused: [],
      }}
      error=""
      partitions={partitions}
      onRefresh={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "取消 Movies" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { task_cancel: { id: "task-one" } },
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: /需处理/ }));
  expect(screen.getByRole("button", { name: "重试 Backup" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(screen.getByText("断开")).toBeTruthy();
});
test("shows an honest empty state and retains rows on disconnect", () => {
  const view = render(
    <Tasks
      data={{ tasks: [], automatic: true, paused: [] }}
      error=""
      partitions={partitions}
      onRefresh={vi.fn()}
    />,
  );
  expect(screen.getByText("当前没有任务")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
  view.rerender(
    <Tasks
      data={{ tasks: [row], automatic: true, paused: [] }}
      error="服务未连接"
      partitions={partitions}
      onRefresh={vi.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: "Movies" })).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("服务未连接");
  expect(screen.getByRole("button", { name: "取消 Movies" })).toHaveProperty(
    "disabled",
    true,
  );
});
