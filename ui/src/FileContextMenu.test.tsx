import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import FileContextMenu from "./FileContextMenu";
afterEach(cleanup);
test("an unavailable background menu can still receive Escape", () => {
  const close = vi.fn();
  render(
    <FileContextMenu
      x={0}
      y={0}
      count={0}
      online={false}
      singleFile={false}
      sameParent
      onAction={vi.fn()}
      onClose={close}
    />,
  );
  expect(document.activeElement).toBe(screen.getByRole("menu"));
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  expect(close).toHaveBeenCalledOnce();
});
test("keeps keyboard position across rerenders and restores the invoking focus", () => {
  const invoker = document.createElement("button");
  document.body.append(invoker);
  invoker.focus();
  const props = {
    x: 0,
    y: 0,
    count: 1,
    online: true,
    singleFile: true,
    sameParent: true,
    onAction: vi.fn(),
  };
  const view = render(<FileContextMenu {...props} onClose={vi.fn()} />);
  fireEvent.keyDown(screen.getByRole("menu"), { key: "End" });
  expect(document.activeElement?.textContent).toBe("更多...");
  view.rerender(<FileContextMenu {...props} onClose={vi.fn()} />);
  expect(document.activeElement?.textContent).toBe("更多...");
  view.unmount();
  expect(document.activeElement).toBe(invoker);
  invoker.remove();
});
test("keeps native menu last and unavailable file operations disabled", () => {
  render(
    <FileContextMenu
      x={20}
      y={30}
      count={2}
      online={false}
      singleFile={false}
      sameParent={false}
      onAction={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  const items = screen.getAllByRole("menuitem");
  expect(items.at(-1)?.textContent).toBe("更多...");
  expect(
    (screen.getByRole("menuitem", { name: /复制文件/ }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("menuitem", { name: "复制名称" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});
test("keyboard escape closes the menu without triggering an operation", () => {
  const close = vi.fn();
  const action = vi.fn();
  render(
    <FileContextMenu
      x={0}
      y={0}
      count={1}
      online
      singleFile
      sameParent
      onAction={action}
      onClose={close}
    />,
  );
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  expect(close).toHaveBeenCalledOnce();
  expect(action).not.toHaveBeenCalled();
});
