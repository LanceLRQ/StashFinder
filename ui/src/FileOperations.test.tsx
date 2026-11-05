import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import FileOperations, { useFileOperations } from "./FileOperations";
import type { SelectionInput } from "./file-operations";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});
const selection: SelectionInput = {
  origin: { kind: "search", id: 1, range: [0, 0] },
  entries: [],
  positions: [],
};
function Buttons({ changed }: { changed?: () => void }) {
  const operations = useFileOperations();
  return (
    <>
      <button onClick={() => operations.run(selection, "copy")}>
        复制样本
      </button>
      <button onClick={() => operations.run(selection, "delete")}>
        永久删除样本
      </button>
      <button
        onClick={() =>
          operations.run(selection, { rename: "new.txt" }, changed)
        }
      >
        改名样本
      </button>
    </>
  );
}
test("permanent deletion cannot run before confirmation", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "file_prepare"
      ? {
          id: 1,
          count: 1,
          names: ["Test / sample.txt"],
          same_parent: true,
          single_file: true,
        }
      : undefined,
  );
  render(
    <FileOperations>
      <Buttons />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("永久删除样本"));
  await screen.findByRole("alertdialog");
  expect(invoke).not.toHaveBeenCalledWith("file_run", expect.anything());
  fireEvent.click(screen.getByText("取消"));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("file_cancel"));
  expect(invoke).not.toHaveBeenCalledWith("file_run", expect.anything());
});
test("deletion confirmation traps focus and Escape cancels without running", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "file_prepare"
      ? {
          id: 1,
          count: 1,
          names: ["sample"],
          same_parent: true,
          single_file: true,
        }
      : undefined,
  );
  render(
    <FileOperations>
      <Buttons />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("永久删除样本"));
  const dialog = await screen.findByRole("alertdialog");
  const cancel = screen.getByText("取消");
  expect(document.activeElement).toBe(cancel);
  fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
  expect(document.activeElement).toBe(screen.getByText("确认永久删除"));
  fireEvent.keyDown(dialog, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(invoke).not.toHaveBeenCalledWith("file_run", expect.anything());
});
test("a cancelled preparation cannot unlock a newer pending preparation", async () => {
  const failures: ((e: Error) => void)[] = [];
  vi.mocked(invoke).mockImplementation((command) =>
    command === "file_prepare"
      ? new Promise((_, reject) => failures.push(reject))
      : Promise.resolve(
          command === "file_status"
            ? { id: 2, phase: "preparing", total: 0, processed: 0, error: null }
            : undefined,
        ),
  );
  render(
    <FileOperations>
      <Buttons />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("复制样本"));
  fireEvent.click(screen.getByText("取消"));
  await screen.findByText("操作已取消");
  fireEvent.click(screen.getByText("复制样本"));
  failures[0](Error("old failed"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  fireEvent.click(screen.getByText("复制样本"));
  expect(
    vi.mocked(invoke).mock.calls.filter(([c]) => c === "file_prepare"),
  ).toHaveLength(2);
});
test("partial writes still refresh the view and expose item results", async () => {
  const changed = vi.fn();
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "file_prepare")
      return {
        id: 9,
        count: 2,
        names: ["first", "second"],
        same_parent: true,
        single_file: false,
      };
    if (command === "file_run") throw Error("完成 1 项，失败 1 项");
    if (command === "file_report")
      return {
        id: 9,
        total: 2,
        rows: [
          {
            index: 0,
            source: "first",
            state: "completed",
            destination: null,
            error: null,
            note: null,
          },
          {
            index: 1,
            source: "second",
            state: "failed",
            destination: null,
            error: "拒绝访问",
            note: null,
          },
        ],
        next: null,
      };
    return undefined;
  });
  render(
    <FileOperations>
      <Buttons changed={changed} />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("改名样本"));
  await screen.findByRole("alert");
  await waitFor(() => expect(changed).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByText("查看结果"));
  expect(await screen.findByText("second")).toBeTruthy();
  expect(screen.getByText("拒绝访问")).toBeTruthy();
});
