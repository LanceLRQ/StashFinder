import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import IndexControls from "./IndexControls";
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
const status = {
  generation: null,
  active_generation: null,
  phase: "not_indexed",
  started: null,
  completed: null,
  records: 0,
  error: null,
};
function mockStatus(value: object) {
  vi.mocked(invoke).mockImplementation(async (_, args: any) =>
    args.action.index_state
      ? { kind: "index", data: value }
      : { kind: "accepted" },
  );
}
test("journal creation requires an explicit choice and confirmation", async () => {
  mockStatus(status);
  render(<IndexControls partitionId="p" name="Movies" eligible />);
  expect(await screen.findByText("尚未索引")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "索引 Movies" }));
  expect(
    screen.getByRole("radio", { name: "使用已有 USN 日志" }),
  ).toHaveProperty("checked", true);
  fireEvent.click(screen.getByRole("radio", { name: "创建 USN 日志并索引" }));
  expect(screen.getByRole("button", { name: "开始索引" })).toHaveProperty(
    "disabled",
    true,
  );
  fireEvent.click(screen.getByRole("checkbox", { name: /允许在这个分区创建/ }));
  fireEvent.click(screen.getByRole("button", { name: "开始索引" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { index_start: { partition_id: "p", mode: "create_journal" } },
    }),
  );
});
test("offline partitions cannot start scans and failed rebuild preserves published status", async () => {
  mockStatus({
    ...status,
    phase: "failed",
    generation: 2,
    active_generation: 1,
    error: "日志断档",
  });
  render(<IndexControls partitionId="p" name="Movies" eligible={false} />);
  expect(await screen.findByText("日志断档")).toBeTruthy();
  expect(screen.getByText(/仍可搜索旧索引/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "索引 Movies" })).toHaveProperty(
    "disabled",
    true,
  );
});
test("running index can be cancelled", async () => {
  mockStatus({ ...status, phase: "scanning" });
  render(<IndexControls partitionId="p" name="Movies" eligible />);
  fireEvent.click(
    await screen.findByRole("button", { name: "取消索引 Movies" }),
  );
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: "index_cancel",
    }),
  );
});
