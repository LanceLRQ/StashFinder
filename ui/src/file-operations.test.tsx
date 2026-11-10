import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { fileKey } from "./file-operations";
afterEach(cleanup);
test("file shortcuts leave text editing and IME composition untouched", () => {
  const dispatch = vi.fn();
  render(
    <div onKeyDown={(e) => fileKey(e, dispatch)}>
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
  expect(dispatch).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("button"), {
    key: "Delete",
    shiftKey: true,
  });
  expect(dispatch).toHaveBeenLastCalledWith("delete");
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "c", ctrlKey: true });
  expect(dispatch).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(screen.getByRole("button"), { key: "c", ctrlKey: true });
  expect(dispatch).toHaveBeenLastCalledWith("copy");
});
