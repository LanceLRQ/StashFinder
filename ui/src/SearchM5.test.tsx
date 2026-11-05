import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Search from "./Search";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});
function mock() {
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) {
      const { offset, limit } = args.action.search_batch;
      return {
        kind: "search_page",
        data: {
          id: 1,
          done: true,
          total: 10000000,
          groups: 3,
          missing: 2,
          next: offset + limit,
          rows: Array.from({ length: Math.min(limit, 20) }, (_, i) => ({
            partition_id: "p",
            filesystem_id: "fs",
            partition_name: "Movies",
            connection: "offline",
            mounts: [],
            generation: 1,
            partial: false,
            indexed_at: null,
            needs_validation: false,
            hard_link: false,
            duplicate_group: 1,
            file: {
              entry: [i],
              parent: [0],
              object: [i],
              name: `file-${offset + i}.txt`,
              path: `folder/file-${offset + i}.txt`,
              kind: "file",
              size: "0",
              created: null,
              modified: null,
              attributes: 0,
            },
          })),
          unavailable: [],
          error: null,
        },
      };
    }
    return { kind: "accepted" };
  });
}
test("uses the same backend options for type, duplicates and match settings", async () => {
  mock();
  render(<Search partitions={[]} />);
  fireEvent.click(screen.getByText("筛选"));
  fireEvent.change(screen.getByLabelText("重复项"), {
    target: { value: "name;size" },
  });
  fireEvent.click(screen.getByLabelText("区分大小写"));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_start: expect.objectContaining({
          options: expect.objectContaining({
            case_sensitive: true,
            duplicate: ["name", "size"],
          }),
        }),
      },
    }),
  );
});
test("keeps DOM rows bounded and requests distant result positions", async () => {
  mock();
  render(<Search partitions={[]} />);
  await screen.findByText("file-0.txt");
  expect(screen.getAllByRole("row").length).toBeLessThan(70);
  const list = screen.getByLabelText("搜索结果列表");
  fireEvent.scroll(list, { target: { scrollTop: 640000 } });
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: {
        search_batch: expect.objectContaining({ offset: expect.any(Number) }),
      },
    }),
  );
  await waitFor(() =>
    expect(
      screen.queryAllByText(/file-[1-9][0-9]{4,}.txt/).length,
    ).toBeGreaterThan(0),
  );
  expect(screen.getAllByRole("row").length).toBeLessThan(70);
  fireEvent.keyDown(list, { ctrlKey: true, key: "a" });
  expect(await screen.findByText("已选择 10,000,000 项")).toBeTruthy();
});
test("reports a failed cancellation request without an unhandled rejection", async () => {
  vi.mocked(invoke).mockImplementation(async (_, args: any) => {
    if (args.action.search_start) return { kind: "search_started", data: 1 };
    if (args.action.search_batch) return new Promise(() => {});
    if (args.action.search_cancel) throw Error("后台连接已断开");
    return { kind: "accepted" };
  });
  render(<Search partitions={[]} />);
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { search_batch: { id: 1, offset: 0, limit: 32 } },
    }),
  );
  fireEvent.click(screen.getByText("取消搜索"));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe(
      "取消请求失败：后台连接已断开",
    ),
  );
});
