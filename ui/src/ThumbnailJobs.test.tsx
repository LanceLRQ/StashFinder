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
import { ThumbnailProgress } from "./ThumbnailJobs";
import ThumbnailJobs from "./ThumbnailJobs";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test.each([
  ["请先在「设置 → 图片缓存」选择预生成的文件类型", "选择文件类型"],
  ["已有缩略图任务在运行", "查看运行中的任务"],
])("offers the recovery action for %s", async (failure, label) => {
  vi.mocked(invoke).mockRejectedValue(failure);
  const recover = vi.fn();
  render(
    <ThumbnailJobs
      scope={{ partitionId: "p", filesystemId: "fs", label: "照片" }}
      onClose={() => {}}
      onStarted={() => {}}
      onRecovery={recover}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "开始" }));
  fireEvent.click(await screen.findByRole("button", { name: label }));
  expect(recover).toHaveBeenCalledWith(
    label === "选择文件类型" ? "cache" : "tasks",
  );
});

test("opens a folder job with skip selected and shows safe progress after requesting a rebuild", async () => {
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
          total: 10,
          saved: 2,
          skipped: 1,
          unsupported: 1,
          failed: 0,
          error: "系统位图尺寸无效",
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
        {!!visuals.thumbnailTaskRevision && (
          <ThumbnailProgress revision={visuals.thumbnailTaskRevision} />
        )}
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
  const progress = await screen.findByRole("progressbar");
  expect(progress.getAttribute("value")).toBe("4");
  expect(progress.getAttribute("max")).toBe("10");
  expect(screen.queryByText("系统位图尺寸无效")).toBeNull();
  expect(invoke).toHaveBeenCalledWith("thumbnail_start", {
    options: { ...scope, rebuild: true },
  });
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "暂停" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("thumbnail_control", {
      action: "pause",
    }),
  );
});
