import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import SearchExport from "./SearchExport";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("uses the fixed selection, reports progress and cancels a running export", async () => {
  let finish: (written: number | null) => void = () => {};
  const unlisten = vi.fn();
  const onClose = vi.fn();
  vi.mocked(listen).mockImplementation(async (_, callback) => {
    callback({
      event: "search-export-progress",
      id: 1,
      payload: [0, 3] as never,
    });
    return unlisten;
  });
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "export_search_csv"
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : undefined,
  );
  render(
    <SearchExport
      snapshot={{
        id: 8,
        total: 100,
        spans: [
          [2, 4],
          [99, 100],
        ],
        count: 3,
      }}
      onClose={onClose}
    />,
  );
  expect(screen.getByRole("radio", { name: /所选结果/ })).toHaveProperty(
    "checked",
    true,
  );
  fireEvent.click(screen.getByRole("button", { name: "保存 CSV…" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("export_search_csv", {
      id: 8,
      total: 100,
      spans: [
        [2, 4],
        [99, 100],
      ],
    }),
  );
  expect(screen.getByText("已写入 0 / 3 条结果")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(invoke).toHaveBeenCalledWith("cancel_search_export");
  await act(async () => finish(null));
  expect(onClose).toHaveBeenCalledOnce();
  expect(unlisten).toHaveBeenCalledOnce();
});
