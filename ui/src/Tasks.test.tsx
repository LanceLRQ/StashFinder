import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import Tasks from "./Tasks";

afterEach(cleanup);

test("shows unknown counters and an in-place retry for an initial task error", () => {
  const refresh = vi.fn();
  render(
    <Tasks
      data={null}
      error="后台不可用"
      partitions={[]}
      onRefresh={refresh}
      onChooseIndex={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: /最近动态.*—/ })).toBeTruthy();
  expect(screen.queryByText("正在读取任务…")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(refresh).toHaveBeenCalled();
});

test("keeps routine automatic indexing out of the list and tab counts", () => {
  const states = [
    ["auto-running", "running", true],
    ["auto-queued", "queued", true],
    ["auto-waiting", "waiting", true],
    ["auto-completed", "completed", true],
    ["auto-failed", "failed", true],
    ["needs-choice", "needs_confirmation", true],
    ["manual-running", "running", false],
    ["manual-completed", "completed", false],
  ] as const;
  const tasks = states.map(([id, state, automatic]) => ({
    id,
    partition_id: id,
    kind: "sync",
    mode: "journal" as const,
    state,
    automatic,
    created: 1,
    updated: 1,
    error: null,
  }));
  render(
    <Tasks
      data={{ tasks, automatic: true, paused: [], indexes: {} }}
      error=""
      partitions={[]}
      onRefresh={() => {}}
      onChooseIndex={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: /最近动态.*4/ })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /当前.*1/ }));
  expect(screen.getByText("manual-running")).toBeTruthy();
  expect(screen.queryByText("auto-running")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /历史.*1/ }));
  expect(screen.getByText("manual-completed")).toBeTruthy();
  expect(screen.queryByText("auto-completed")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /需处理.*2/ }));
  expect(screen.getByText("auto-failed")).toBeTruthy();
  expect(screen.getByText("needs-choice")).toBeTruthy();
});
