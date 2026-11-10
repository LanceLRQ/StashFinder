import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import WindowChrome from "./WindowChrome";

const windowApi = vi.hoisted(() => ({
  minimize: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  close: vi.fn(async () => {}),
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async (_callback: () => void) => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowApi,
}));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("keeps interactive tabs out of the drag region and uses native window actions", async () => {
  vi.stubGlobal("__TAURI_INTERNALS__", {});
  const view = render(
    <WindowChrome>
      <button>标签</button>
    </WindowChrome>,
  );
  expect(
    screen.getByRole("button", { name: "标签" }).closest(".caption-drag"),
  ).toBeNull();
  expect(view.container.querySelector(".caption-drag")?.children).toHaveLength(
    0,
  );
  fireEvent.click(screen.getByRole("button", { name: "最小化" }));
  windowApi.isMaximized.mockResolvedValue(true);
  fireEvent.click(screen.getByRole("button", { name: "最大化" }));
  await screen.findByRole("button", { name: "还原" });
  fireEvent.click(screen.getByRole("button", { name: "关闭窗口" }));
  expect(windowApi.minimize).toHaveBeenCalledTimes(1);
  expect(windowApi.toggleMaximize).toHaveBeenCalledTimes(1);
  expect(windowApi.close).toHaveBeenCalledTimes(1);
});
