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
  { id: "one", name: "Movies", connection: "online", canChooseIndex: true },
  { id: "two", name: "Backup", connection: "offline", canChooseIndex: false },
];
test("defaults to the five most recently updated tasks across groups", () => {
  const names = ["Oldest", "Third", "Newest", "Fifth", "Second", "Fourth"];
  render(
    <Tasks
      data={{
        tasks: [1, 4, 6, 2, 5, 3].map((updated, i) => ({
          ...row,
          id: names[i],
          partition_id: names[i],
          state: i % 3 === 0 ? "running" : i % 3 === 1 ? "failed" : "completed",
          updated,
        })),
        automatic: true,
        paused: [],
      }}
      error=""
      partitions={[]}
      onRefresh={vi.fn()}
      onChooseIndex={vi.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: /最近动态/ })).toHaveProperty(
    "ariaPressed",
    "true",
  );
  expect(
    within(screen.getByRole("table"))
      .getAllByRole("button", {
        name: /^(Oldest|Third|Newest|Fifth|Second|Fourth)$/,
      })
      .map((button) => button.textContent),
  ).toEqual(["Newest", "Second", "Third", "Fourth", "Fifth"]);
  fireEvent.click(screen.getByRole("button", { name: /当前/ }));
  expect(screen.getByRole("button", { name: "Oldest" })).toBeTruthy();
});

test("focuses and expands the requested task when its snapshot arrives", () => {
  const props = {
    error: "",
    partitions,
    onRefresh: vi.fn(),
    onChooseIndex: vi.fn(),
    focusTask: row.id,
  };
  const view = render(<Tasks {...props} data={null} />);
  view.rerender(
    <Tasks
      {...props}
      data={{
        tasks: [{ ...row, state: "needs_confirmation" }],
        automatic: true,
        paused: [],
      }}
    />,
  );
  expect(screen.getByRole("button", { name: /需处理/ })).toHaveProperty(
    "ariaPressed",
    "true",
  );
  const target = screen.getByRole("button", { name: "Movies" });
  expect(document.activeElement).toBe(target);
  expect(target).toHaveProperty("ariaExpanded", "true");
});
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
      onChooseIndex={vi.fn()}
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
      onChooseIndex={vi.fn()}
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
      onChooseIndex={vi.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: "Movies" })).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("服务未连接");
  expect(screen.getByRole("button", { name: "取消 Movies" })).toHaveProperty(
    "disabled",
    true,
  );
});
test("an online task waiting for a journal choice can be resolved in place", () => {
  const onChooseIndex = vi.fn();
  render(
    <Tasks
      data={{
        tasks: [
          { ...row, state: "needs_confirmation" },
          {
            ...row,
            id: "two-task",
            partition_id: "two",
            state: "needs_confirmation",
          },
        ],
        automatic: true,
        paused: [],
      }}
      error=""
      focusTask={row.id}
      partitions={partitions}
      onRefresh={vi.fn()}
      onChooseIndex={onChooseIndex}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "选择索引方式 Movies" }));
  expect(onChooseIndex).toHaveBeenCalledWith("one");
  expect(
    screen.getByRole("button", { name: "选择索引方式 Backup" }),
  ).toHaveProperty("disabled", true);
});
