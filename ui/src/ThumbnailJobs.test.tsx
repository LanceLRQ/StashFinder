import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Visuals, { useVisuals } from "./Visuals";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("opens a folder job with skip selected and reopens its progress after requesting a rebuild", async () => {
  const scope = {
    partitionId: "p",
    filesystemId: "fs",
    generation: "2",
    folder: [3],
    label: "照片",
  };
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "thumbnail_status"
      ? {
          state: "running",
          label: scope.label,
          discovered: 5,
          saved: 2,
          skipped: 1,
          unsupported: 1,
          failed: 0,
        }
      : undefined,
  );
  function Entry() {
    const visuals = useVisuals()!;
    return (
      <>
        <button onClick={() => visuals.openThumbnails(scope)}>
          开始目录任务
        </button>
        <button onClick={() => visuals.openThumbnails("progress")}>
          查看任务
        </button>
      </>
    );
  }
  render(
    <Visuals>
      <Entry />
    </Visuals>,
  );
  fireEvent.click(screen.getByRole("button", { name: "开始目录任务" }));
  expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe(
    "skip",
  );
  fireEvent.change(screen.getByRole("combobox"), {
    target: { value: "rebuild" },
  });
  fireEvent.click(screen.getByRole("button", { name: "开始" }));
  await screen.findByText("已处理 4 项，已发现 5 项");
  expect(invoke).toHaveBeenCalledWith("thumbnail_start", {
    options: { ...scope, rebuild: true },
  });
  fireEvent.click(screen.getByRole("button", { name: "关闭" }));
  fireEvent.click(screen.getByRole("button", { name: "查看任务" }));
  await screen.findByText("已处理 4 项，已发现 5 项");
  fireEvent.click(screen.getByRole("button", { name: "暂停" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("thumbnail_control", {
      action: "pause",
    }),
  );
});
