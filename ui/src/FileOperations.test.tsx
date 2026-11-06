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
      <button
        onClick={() => operations.run(selection, { more: { x: 20, y: 30 } })}
      >
        更多样本
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
      <button
        onClick={() =>
          operations.paste(
            {
              partition_id: "p",
              filesystem_id: "fs",
              generation: "1",
              revision: "1",
              entry: [1],
            },
            changed,
          )
        }
      >
        粘贴样本
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

test("native menu preparation and completion stay quiet without unlocking other operations", async () => {
  let finish!: (value: string) => void;
  vi.mocked(invoke).mockImplementation((command) => {
    if (command === "file_prepare")
      return Promise.resolve({
        id: 11,
        count: 2,
        names: ["a", "b"],
        same_parent: true,
        single_file: false,
      });
    if (command === "file_run")
      return new Promise<string>((resolve) => {
        finish = resolve;
      });
    return Promise.resolve(null);
  });
  render(
    <FileOperations>
      <Buttons />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("更多样本"));
  expect(screen.queryByLabelText("文件操作状态")).toBeNull();
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("file_run", {
      id: 11,
      action: { more: { x: 20, y: 30 } },
    }),
  );
  expect(screen.queryByLabelText("文件操作状态")).toBeNull();
  fireEvent.click(screen.getByText("复制样本"));
  expect(
    vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === "file_prepare"),
  ).toHaveLength(1);
  finish("已关闭 Windows 菜单");
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("file_report", { id: 11, offset: 0 }),
  );
  expect(screen.queryByLabelText("文件操作状态")).toBeNull();
  fireEvent.click(screen.getByText("复制样本"));
  expect(await screen.findByLabelText("文件操作状态")).toBeTruthy();
});

test.each(["file_prepare", "file_run"])(
  "native menu errors remain visible at %s",
  async (failure) => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === failure) throw Error("无法打开 Windows 菜单");
      if (command === "file_prepare")
        return {
          id: 12,
          count: 1,
          names: ["sample"],
          same_parent: true,
          single_file: true,
        };
      return null;
    });
    render(
      <FileOperations>
        <Buttons />
      </FileOperations>,
    );
    fireEvent.click(screen.getByText("更多样本"));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "无法打开 Windows 菜单",
    );
  },
);

test("quiet native menus still expose the in-progress window-close confirmation", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "file_prepare")
      return {
        id: 13,
        count: 1,
        names: ["sample"],
        same_parent: true,
        single_file: true,
      };
    if (command === "file_run") return new Promise(() => {});
    if (command === "file_status")
      return {
        id: 13,
        phase: "executing",
        total: 1,
        processed: 0,
        error: null,
        close_requested: true,
      };
    return null;
  });
  render(
    <FileOperations>
      <Buttons />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("更多样本"));
  expect(
    await screen.findByRole("alertdialog", { name: "文件操作仍在进行" }),
  ).toBeTruthy();
  expect(screen.queryByLabelText("文件操作状态")).toBeNull();
  fireEvent.click(screen.getByText("等待完成"));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("file_keep_open"));
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
        completed: 1,
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

test("cancelling a native paste with no writes does not mark the index as pending", async () => {
  const changed = vi.fn();
  const message = "完成 0 项，失败 0 项，跳过 1 项，未执行 0 项；操作已取消";
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "file_paste") return message;
    if (command === "file_report")
      return {
        id: 10,
        total: 1,
        completed: 0,
        rows: [
          {
            index: 0,
            source: "sample",
            state: "skipped",
            destination: null,
            error: null,
            note: "用户已取消，未完成",
          },
        ],
        next: null,
      };
  });
  render(
    <FileOperations>
      <Buttons changed={changed} />
    </FileOperations>,
  );
  fireEvent.click(screen.getByText("粘贴样本"));
  await screen.findByText(message);
  await screen.findByText("查看结果");
  expect(changed).not.toHaveBeenCalled();
  expect(screen.queryByRole("alert")).toBeNull();
});
