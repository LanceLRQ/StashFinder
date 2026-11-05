import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { fileKey } from "./file-operations";
afterEach(cleanup);
test("file shortcuts leave text editing and IME composition untouched", () => {
  const copy = vi.fn(),
    remove = vi.fn();
  const actions = {
    copy,
    remove,
    cut: vi.fn(),
    rename: vi.fn(),
    properties: vi.fn(),
    menu: vi.fn(),
  };
  render(
    <div onKeyDown={(e) => fileKey(e, actions)}>
      <input aria-label="名称" />
      <button>列表</button>
    </div>,
  );
  fireEvent.keyDown(screen.getByRole("textbox"), {
    key: "Delete",
    shiftKey: true,
  });
  fireEvent.keyDown(screen.getByRole("button"), {
    key: "Delete",
    isComposing: true,
  });
  expect(remove).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("button"), {
    key: "Delete",
    shiftKey: true,
  });
  expect(remove).toHaveBeenCalledWith(true);
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "c", ctrlKey: true });
  expect(copy).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("button"), { key: "c", ctrlKey: true });
  expect(copy).toHaveBeenCalledOnce();
});
