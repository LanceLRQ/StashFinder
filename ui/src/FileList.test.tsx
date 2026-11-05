import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import FileList from "./FileList";
afterEach(cleanup);
test("virtualizes loaded directory rows and keeps selection indexes across scroll windows", () => {
  const select = vi.fn();
  const rows = Array.from({ length: 3000 }, (_, i) => ({
    entry: [i % 256, Math.floor(i / 256)],
    parent: [0],
    object: [i % 256, Math.floor(i / 256)],
    name: `file-${i}.txt`,
    path: `file-${i}.txt`,
    kind: "file",
    attributes: 0,
    size: "0",
    created: null,
    modified: null,
  }));
  render(
    <FileList
      rows={rows}
      selection={[]}
      onSelect={select}
      onOpen={() => {}}
      next={null}
      busy={false}
      onMore={() => {}}
    />,
  );
  expect(screen.getAllByRole("row").length).toBeLessThan(60);
  const list = screen.getByLabelText("目录文件列表");
  fireEvent.scroll(list, { target: { scrollTop: 34000 } });
  fireEvent.click(screen.getByText("file-1000.txt"));
  expect(select).toHaveBeenLastCalledWith([rows[1000].entry.join(",")]);
  fireEvent.keyDown(list, { ctrlKey: true, key: "a" });
  expect(select.mock.lastCall?.[0]).toHaveLength(3000);
  expect(screen.getAllByRole("row").length).toBeLessThan(60);
});
