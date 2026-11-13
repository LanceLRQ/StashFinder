import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import AppMenu from "./AppMenu";

afterEach(cleanup);

test("enters and leaves a submenu one level at a time", () => {
  const run = vi.fn();
  render(
    <AppMenu
      groups={[
        {
          label: "查看",
          accessKey: "v",
          items: [
            {
              label: "布局",
              items: [{ label: "详情", run, checked: true, radio: true }],
            },
          ],
        },
      ]}
    />,
  );
  const view = screen.getByRole("button", { name: "查看" });
  view.focus();
  fireEvent.keyDown(view, { key: "ArrowDown" });
  const layout = screen.getByRole("menuitem", { name: "布局" });
  fireEvent.keyDown(layout, { key: "ArrowRight" });
  const details = screen.getByRole("menuitemradio", { name: "详情" });
  expect(document.activeElement).toBe(details);
  fireEvent.keyDown(details, { key: "Escape" });
  expect(screen.queryByRole("menu", { name: "布局" })).toBeNull();
  expect(document.activeElement).toBe(layout);
  fireEvent.keyDown(layout, { key: "Enter" });
  fireEvent.keyDown(document.activeElement!, { key: "Enter" });
  expect(run).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(view);
});

test("opens Edit with Alt+E, navigates enabled commands and restores focus", () => {
  const run = vi.fn();
  const unavailable = vi.fn();
  render(
    <>
      <button>文件列表</button>
      <AppMenu
        groups={[
          {
            label: "编辑",
            accessKey: "e",
            items: [
              { label: "剪切", run: unavailable, disabled: true },
              { label: "复制名称", run },
              { label: "复制路径", run },
            ],
          },
        ]}
      />
    </>,
  );
  const source = screen.getByRole("button", { name: "文件列表" });
  source.focus();
  fireEvent.keyDown(source, { altKey: true, key: "e" });
  expect(document.activeElement).toBe(
    screen.getByRole("menuitem", { name: "复制名称" }),
  );
  fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
  const path = screen.getByRole("menuitem", { name: "复制路径" });
  expect(document.activeElement).toBe(path);
  fireEvent.keyDown(path, { key: "Home" });
  fireEvent.keyDown(document.activeElement!, { key: "End" });
  expect(document.activeElement).toBe(path);
  fireEvent.keyDown(path, { key: "Enter" });
  expect(run).toHaveBeenCalledTimes(1);
  expect(unavailable).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(source);
  fireEvent.keyDown(source, { altKey: true, key: "e" });
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  expect(screen.queryByRole("menu")).toBeNull();
  expect(document.activeElement).toBe(source);
});
