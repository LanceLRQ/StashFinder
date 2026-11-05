import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import Desktop from "./Desktop";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeEach(() => {
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "preferences_get")
      return {
        version: 1,
        collapsed: false,
        start_page: "overview",
        include_path: true,
      };
    if (command === "preferences_set") return args.preferences;
    if (command === "asset_snapshot")
      return {
        version: 1,
        busy: false,
        error: null,
        assets: [],
        candidates: [],
      };
    if (command === "service_status") throw new Error("服务未启动");
    if (command === "app_version") return "0.1.0";
    if (args?.action?.search_start) return { kind: "search_started", data: 1 };
    if (args?.action?.search_page)
      return {
        kind: "search_page",
        data: {
          id: 1,
          done: true,
          row: null,
          next: null,
          more: false,
          error: null,
          unavailable: [],
        },
      };
    return { kind: "accepted" };
  });
});
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("keeps navigation usable and search input when switching pages and collapsing", async () => {
  render(<Desktop />);
  const overview = await screen.findByRole("button", { name: "概览" });
  expect(overview.getAttribute("aria-current")).toBe("page");
  for (const name of ["文件", "任务", "设置"])
    expect(screen.getByRole("button", { name })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "文件" }));
  fireEvent.change(screen.getByRole("searchbox", { name: "搜索文件或路径" }), {
    target: { value: "aaa bbb" },
  });
  await screen.findByText("没有匹配的文件");
  fireEvent.click(overview);
  fireEvent.click(screen.getByRole("button", { name: "收起功能栏" }));
  await screen.findByRole("button", { name: "展开功能栏" });
  fireEvent.click(screen.getByRole("button", { name: "文件" }));
  expect(
    screen.getByRole("searchbox", { name: "搜索文件或路径" }),
  ).toHaveProperty("value", "aaa bbb");
  expect(screen.getAllByRole("main")).toHaveLength(1);
});

test("restores startup and search preferences before mounting the first search", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args) =>
    command === "preferences_get"
      ? Promise.resolve({
          version: 1,
          collapsed: true,
          start_page: "files",
          include_path: false,
        })
      : original(command, args),
  );
  render(<Desktop />);
  await screen.findByRole("button", { name: "展开功能栏" });
  expect(
    screen.getByRole("button", { name: "文件" }).getAttribute("aria-current"),
  ).toBe("page");
  expect(screen.getByRole("checkbox", { name: "仅文件名" })).toHaveProperty(
    "checked",
    true,
  );
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("index_request", {
      action: { search_start: { text: "", names_only: true, partitions: [] } },
    }),
  );
});

test("keeps settings reachable when the service and preference storage fail", async () => {
  vi.mocked(invoke).mockRejectedValue("无法读取");
  render(<Desktop />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));
  expect(await screen.findByLabelText("启动页面")).toBeTruthy();
  expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole("button", { name: "收起功能栏" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "收起功能栏" })).toHaveProperty(
      "disabled",
      false,
    ),
  );
  expect(screen.queryByRole("button", { name: "展开功能栏" })).toBeNull();
});

test("submitting the same overview query again refreshes results", async () => {
  render(<Desktop />);
  const overview = await screen.findByRole("button", { name: "概览" });
  const search = screen.getByRole("searchbox", { name: "搜索所有文件" });
  fireEvent.change(search, { target: { value: "same query" } });
  fireEvent.submit(search.closest("form")!);
  await screen.findByText("没有匹配的文件");
  const first = vi
    .mocked(invoke)
    .mock.calls.filter(([, args]: any) => args?.action?.search_start).length;
  fireEvent.click(overview);
  fireEvent.submit(search.closest("form")!);
  await waitFor(() =>
    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter(([, args]: any) => args?.action?.search_start)
        .length,
    ).toBe(first + 1),
  );
});
