import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Visuals from "./Visuals";
import FileIcon from "./FileIcon";
import CacheSettings, { type CacheSettingsControls } from "./CacheSettings";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));
beforeEach(() => vi.mocked(listen).mockResolvedValue(vi.fn()));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

function CacheForm() {
  const controls = useRef<CacheSettingsControls>(null);
  return (
    <>
      <CacheSettings ref={controls} />
      <button onClick={() => void controls.current?.save()}>确定</button>
    </>
  );
}

test("saves unlimited capacity and selected types with custom extensions", async () => {
  let status = {
    used_bytes: 42,
    limit_bytes: 2_000_000_000 as number | null,
    free_bytes: 100_000_000,
    files: 1,
    waiting: 0,
    active: 0,
    thumbnail_types: {
      images: true,
      videos: true,
      documents: true,
      extensions: [] as string[],
    },
  };
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "visual_cache_limit")
      status = {
        ...status,
        limit_bytes: (args as { limitBytes: number | null }).limitBytes,
      };
    if (command === "visual_thumbnail_types")
      status = {
        ...status,
        thumbnail_types: (args as { types: typeof status.thumbnail_types })
          .types,
      };
    return status;
  });
  render(<CacheForm />);
  await screen.findByText("42 B / 2 GB");
  fireEvent.click(screen.getByRole("checkbox", { name: "不限容量" }));
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await screen.findByText("42 B / 不限容量");
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(screen.getByText("磁盘剩余 100 MB")).toBeTruthy();
  fireEvent.click(screen.getByRole("checkbox", { name: "视频" }));
  fireEvent.change(screen.getByRole("textbox", { name: "自定义扩展名" }), {
    target: { value: ".JXL, abc" },
  });
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("visual_thumbnail_types", {
      types: {
        images: true,
        videos: false,
        documents: true,
        extensions: [".JXL", "abc"],
      },
    }),
  );
});

test("maintenance progress uses its own operation and never presents an old response as current usage", async () => {
  let progress!: (event: any) => void;
  let finish!: (value: unknown) => void;
  let operationId = "";
  vi.mocked(listen).mockImplementation(async (event, handler: any) => {
    if (event === "visual-cache-progress") progress = handler;
    return vi.fn<() => void>();
  });
  vi.mocked(invoke).mockImplementation((command, args: any) => {
    if (command === "visual_cache_limit") {
      operationId = args.requestId;
      return new Promise((resolve) => {
        finish = resolve;
      });
    }
    return Promise.resolve({
      used_bytes: 100_000_000,
      limit_bytes: 2_000_000_000,
      free_bytes: null,
      thumbnail_types: {
        images: true,
        videos: true,
        documents: true,
        extensions: [],
      },
      files: 5,
      waiting: 0,
      active: 0,
    });
  });
  render(
    <Visuals>
      <CacheForm />
    </Visuals>,
  );
  await screen.findByText("100 MB / 2 GB");
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await waitFor(() => expect(operationId).not.toBe(""));
  await act(async () =>
    progress({
      payload: {
        request_id: "old",
        progress: { processed: 9, total: 10, used_bytes: 1, limit_bytes: 1 },
      },
    }),
  );
  expect(screen.getByText("100 MB / 2 GB")).toBeTruthy();
  await act(async () =>
    progress({
      payload: {
        request_id: operationId,
        progress: {
          processed: 1,
          total: 5,
          used_bytes: 40_000_000,
          limit_bytes: 1_000_000_000,
        },
      },
    }),
  );
  expect(screen.getByText("正在整理 1 / 5")).toBeTruthy();
  expect(screen.getByText("40 MB / 1 GB")).toBeTruthy();
  await act(async () =>
    finish({
      used_bytes: 0,
      limit_bytes: 1_000_000_000,
      free_bytes: null,
      thumbnail_types: {
        images: true,
        videos: true,
        documents: true,
        extensions: [],
      },
      files: 0,
      waiting: 0,
      active: 0,
    }),
  );
  expect(screen.getByText("0 B / 1 GB")).toBeTruthy();
  await act(async () =>
    progress({
      payload: {
        request_id: operationId,
        progress: {
          processed: 2,
          total: 5,
          used_bytes: 30_000_000,
          limit_bytes: 1_000_000_000,
        },
      },
    }),
  );
  expect(screen.getByText("0 B / 1 GB")).toBeTruthy();
});

test("cache controls show real bytes, save the new limit and clear without losing it", async () => {
  let status = {
    used_bytes: 100_000_000,
    limit_bytes: 2_000_000_000,
    free_bytes: null,
    thumbnail_types: {
      images: true,
      videos: true,
      documents: true,
      extensions: [],
    },
    files: 5,
    waiting: 1,
    active: 1,
  };
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "visual_icon")
      return {
        data_url: "data:image/png;base64,aW1hZ2U=",
        source: "system_icon",
        historical: false,
        warning: null,
      };
    if (command === "visual_cache_limit")
      status = {
        ...status,
        limit_bytes: args.limitBytes,
        used_bytes: 30_000_000,
      };
    if (command === "visual_cache_clear")
      status = { ...status, used_bytes: 0, files: 0 };
    return status;
  });
  render(
    <Visuals>
      <FileIcon name="sample.txt" folder={false} />
      <CacheForm />
    </Visuals>,
  );
  const input = await screen.findByRole("spinbutton", {
    name: "图片缓存上限（GB）",
  });
  await screen.findByText("100 MB / 2 GB");
  expect(screen.getByRole("progressbar").getAttribute("value")).toBe(
    "100000000",
  );
  fireEvent.change(input, { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: "确定" }));
  await screen.findByText("30 MB / 1 GB");
  expect(invoke).toHaveBeenCalledWith("visual_cache_limit", {
    limitBytes: 1_000_000_000,
    requestId: expect.any(String),
  });
  fireEvent.click(screen.getByRole("button", { name: "清空缓存" }));
  const dialog = await screen.findByRole("alertdialog", {
    name: "清空图片缓存？",
  });
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "关闭" }),
  );
  fireEvent(dialog, new Event("cancel", { cancelable: true }));
  expect(
    vi
      .mocked(invoke)
      .mock.calls.some(([command]) => command === "visual_cache_clear"),
  ).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "清空缓存" }));
  fireEvent.click(await screen.findByRole("button", { name: "确认清空" }));
  await screen.findByText("0 B / 1 GB");
  expect((input as HTMLInputElement).value).toBe("1");
  await waitFor(() =>
    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter(([command]) => command === "visual_icon"),
    ).toHaveLength(2),
  );
});

test("cache read failures are explicit and do not show zero usage as a success", async () => {
  vi.mocked(invoke).mockRejectedValue(Error("缓存目录无法读取"));
  render(
    <Visuals>
      <CacheSettings />
    </Visuals>,
  );
  expect((await screen.findByRole("alert")).textContent).toBe(
    "缓存目录无法读取",
  );
  expect(screen.queryByRole("progressbar")).toBeNull();
});
