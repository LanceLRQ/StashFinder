import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("cuts without removing a row until refreshed, then marks missing sources unavailable", async () => {
  const sample = {
    id: 0,
    label: "sample.txt",
    path: "D:\\tmp\\sample.txt",
    kind: "文件",
    available: true,
  };
  vi.mocked(invoke).mockResolvedValueOnce([sample]);
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "准备验证样本" }));
  const row = await screen.findByRole("option", { name: /sample.txt/ });
  fireEvent.mouseDown(row, { button: 0 });
  fireEvent.mouseUp(row, { button: 0 });
  vi.mocked(invoke).mockResolvedValueOnce("等待粘贴");
  fireEvent.keyDown(screen.getByRole("listbox"), { key: "x", ctrlKey: true });
  expect(await screen.findByText("等待粘贴")).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith("cut_samples", { ids: [0] });
  expect(screen.getByRole("option", { name: /sample.txt/ })).toBeTruthy();
  vi.mocked(invoke).mockResolvedValueOnce([{ ...sample, available: false }]);
  fireEvent.keyDown(screen.getByRole("listbox"), { key: "F5" });
  expect(
    await screen.findByRole("option", { name: /sample.txt（不可用）/ }),
  ).toBeTruthy();
});

test("requires distinct delete confirmation, allows cancellation and displays rename errors", async () => {
  const sample = {
    id: 0,
    label: "sample.txt",
    path: "D:\\tmp\\sample.txt",
    kind: "文件",
    available: true,
  };
  vi.mocked(invoke).mockResolvedValueOnce([sample]);
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "准备验证样本" }));
  const row = await screen.findByRole("option", { name: /sample.txt/ });
  fireEvent.mouseDown(row, { button: 0 });
  fireEvent.mouseUp(row, { button: 0 });
  fireEvent.keyDown(screen.getByRole("listbox"), { key: "Delete" });
  expect(screen.getByRole("dialog", { name: "移入回收站" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(screen.getByRole("listbox"), {
    key: "Delete",
    shiftKey: true,
  });
  expect(screen.getByRole("dialog", { name: "永久删除" })).toBeTruthy();
  expect(screen.getByText(/不经过回收站/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  fireEvent.keyDown(screen.getByRole("listbox"), { key: "F2" });
  fireEvent.change(screen.getByRole("textbox", { name: "新名称" }), {
    target: { value: "existing.txt" },
  });
  vi.mocked(invoke)
    .mockRejectedValueOnce("目标名称已存在")
    .mockResolvedValueOnce([sample]);
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  expect(await screen.findByText(/目标名称已存在/)).toBeTruthy();
  expect(screen.getByRole("option", { name: /sample.txt/ })).toBeTruthy();
});

test("shows actual service mode and clears stale success on failure", async () => {
  vi.mocked(invoke).mockResolvedValueOnce({
    mode: "foreground",
    process_id: 42,
    registered_partitions: 0,
  });
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "检查连接" }));
  expect(await screen.findByText(/前台验证进程 · PID 42/)).toBeTruthy();
  vi.mocked(invoke).mockRejectedValueOnce("service unavailable");
  fireEvent.click(screen.getByRole("button", { name: "检查连接" }));
  expect(await screen.findByText(/service unavailable/)).toBeTruthy();
  expect(screen.queryByText(/PID 42/)).toBeNull();
});

test("selects rows with Ctrl and Shift and copies IDs using Ctrl+C", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(
    [0, 1, 2, 3].map((id) => ({
      id,
      label: `sample-${id}`,
      path: `path-${id}`,
      kind: "文件",
    })),
  );
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "准备验证样本" }));
  const first = await screen.findByRole("option", { name: /sample-0/ });
  fireEvent.mouseDown(first, { button: 0 });
  fireEvent.mouseUp(first, { button: 0 });
  const third = screen.getByRole("option", { name: /sample-2/ });
  fireEvent.mouseDown(third, { button: 0, shiftKey: true });
  fireEvent.mouseUp(third, { button: 0, shiftKey: true });
  const second = screen.getByRole("option", { name: /sample-1/ });
  fireEvent.mouseDown(second, { button: 0, ctrlKey: true });
  fireEvent.mouseUp(second, { button: 0, ctrlKey: true });
  vi.mocked(invoke).mockRejectedValueOnce("sample unavailable");
  fireEvent.keyDown(screen.getByRole("listbox"), { key: "c", ctrlKey: true });
  await waitFor(() =>
    expect(invoke).toHaveBeenLastCalledWith("copy_samples", { ids: [0, 2] }),
  );
  expect(await screen.findByText(/sample unavailable/)).toBeTruthy();
});

test("drags selected rows directly, preserves the group and cancels click collapse", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(
    [0, 1].map((id) => ({
      id,
      label: `sample-${id}`,
      path: `path-${id}`,
      kind: "文件",
    })),
  );
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "准备验证样本" }));
  const first = await screen.findByRole("option", { name: /sample-0/ });
  const list = screen.getByRole("listbox");
  fireEvent.keyDown(list, { key: "a", ctrlKey: true });
  vi.mocked(invoke).mockResolvedValueOnce("drag cancelled");
  fireEvent.mouseDown(first, { button: 0, clientX: 10, clientY: 10 });
  fireEvent.mouseMove(first, { buttons: 1, clientX: 11, clientY: 11 });
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.mouseMove(first, { buttons: 1, clientX: 20, clientY: 10 });
  fireEvent.mouseMove(first, { buttons: 1, clientX: 25, clientY: 10 });
  fireEvent.mouseUp(first, { button: 0 });
  await screen.findByText("drag cancelled");
  expect(invoke).toHaveBeenLastCalledWith("drag_samples", { ids: [0, 1] });
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(screen.getAllByRole("option", { selected: true })).toHaveLength(2);
  fireEvent.keyDown(list, { key: "Escape" });
  fireEvent.keyDown(list, { key: "c", ctrlKey: true });
  expect(invoke).toHaveBeenCalledTimes(2);
});
