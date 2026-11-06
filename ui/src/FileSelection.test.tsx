import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import FileList from "./FileList";
import VirtualResults from "./VirtualResults";
import RenameInput from "./RenameInput";
import styles from "./file-operations.css?raw";

afterEach(() => {
  cleanup();
  document.head.querySelector("style[data-file-selection]")?.remove();
});

test("file lists suppress text selection while rename and search inputs remain editable", () => {
  const style = document.createElement("style");
  style.dataset.fileSelection = "";
  style.textContent = styles;
  document.head.append(style);
  render(
    <>
      <input type="search" aria-label="搜索" />
      <FileList
        rows={[
          {
            entry: [1],
            parent: [0],
            object: [1],
            name: "sample.txt",
            path: "sample.txt",
            kind: "file",
            attributes: 0,
            size: "1",
            created: null,
            modified: null,
          },
        ]}
        selection={[]}
        onSelect={() => {}}
        onOpen={() => {}}
        next={null}
        busy={false}
        onMore={() => {}}
      />
      <VirtualResults
        total={1}
        revision={0}
        onRange={() => {}}
        onKeyDown={() => {}}
        row={() => (
          <tr key="result">
            <td>result.txt</td>
            <td>
              <RenameInput
                name="rename.txt"
                folder={false}
                onSubmit={() => {}}
                onCancel={() => {}}
              />
            </td>
          </tr>
        )}
      />
    </>,
  );
  expect(
    getComputedStyle(screen.getByText("sample.txt").closest("td")!).userSelect,
  ).toBe("none");
  expect(getComputedStyle(screen.getByText("result.txt")).userSelect).toBe(
    "none",
  );
  const rename = screen.getByLabelText("新名称") as HTMLInputElement;
  expect(getComputedStyle(rename).userSelect).toBe("text");
  expect(rename.selectionStart).toBe(0);
  expect(rename.selectionEnd).toBe(6);
  expect(getComputedStyle(screen.getByLabelText("搜索")).userSelect).not.toBe(
    "none",
  );
});
