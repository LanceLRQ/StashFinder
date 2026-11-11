import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { useState } from "react";
import FileList from "./FileList";
afterEach(cleanup);

const files = Array.from({ length: 12 }, (_, index) => ({
  entry: [index],
  parent: [99],
  object: [index],
  name: `sample-${index}.txt`,
  path: `sample-${index}.txt`,
  kind: "file",
  attributes: 0,
  size: null,
  created: null,
  modified: null,
}));

function measureList(list: HTMLElement, height: number) {
  Object.defineProperties(list, {
    clientHeight: { value: height },
    clientWidth: { value: 660 },
  });
  vi.spyOn(list, "getBoundingClientRect").mockReturnValue(
    DOMRect.fromRect({ width: 660, height }),
  );
  fireEvent.resize(window);
}

test.each([
  ["details", 34, 16, 34],
  ["small", 82, 106, 82],
  ["medium", 108, 146, 108],
  ["large", 176, 210, 176],
] as const)(
  "frames directory items in %s, toggles with Ctrl and discards an unfinished frame",
  (view, rowHeight, edge, gap) => {
    const commit = vi.fn();
    function Harness() {
      const [selection, select] = useState(["0"]);
      return (
        <FileList
          rows={files}
          view={view}
          selection={selection}
          onSelect={(ids) => {
            commit(ids);
            select(ids);
          }}
          onOpen={() => {}}
          next={null}
          busy={false}
          onMore={() => {}}
        />
      );
    }
    render(<Harness />);
    const list = screen.getByLabelText("目录文件列表");
    measureList(list, 480);
    const pointer = { pointerId: 7, pointerType: "mouse", button: 0 };
    const frame = (ctrlKey: boolean) => {
      fireEvent.pointerDown(list, {
        ...pointer,
        clientX: 1,
        clientY: gap + 1,
        ctrlKey,
      });
      fireEvent.pointerMove(list, {
        ...pointer,
        buttons: 1,
        clientX: edge,
        clientY: 5,
        ctrlKey,
      });
    };
    const marked = () =>
      [...list.querySelectorAll('[data-index][aria-selected="true"]')].map(
        (item) => item.getAttribute("data-index"),
      );
    frame(true);
    expect(marked()).toEqual(view === "details" ? ["1"] : []);
    expect(commit).not.toHaveBeenCalled();
    fireEvent.keyDown(list, { key: "Escape" });
    fireEvent.pointerUp(list, pointer);
    expect(marked()).toEqual(["0"]);
    expect(commit).not.toHaveBeenCalled();
    frame(false);
    fireEvent.pointerUp(list, { ...pointer, clientX: edge, clientY: 5 });
    expect(commit).toHaveBeenLastCalledWith(
      view === "details" ? ["0", "1"] : ["0"],
    );
    frame(true);
    fireEvent.pointerUp(list, { ...pointer, clientX: edge, clientY: 5 });
    expect(commit).toHaveBeenLastCalledWith([]);
    expect(list.style.getPropertyValue("--details-row-height")).toBe(
      `${rowHeight}px`,
    );
  },
);

test("cancels a frame when the directory revision changes or pointer capture is lost", () => {
  const commit = vi.fn();
  const renderList = (revision: string) => (
    <FileList
      rows={files}
      selection={["4"]}
      onSelect={commit}
      onOpen={() => {}}
      context={{
        partition_id: "p",
        filesystem_id: "f",
        generation: "1",
        revision,
        entry: [1],
      }}
      next={null}
      busy={false}
      onMore={() => {}}
    />
  );
  const { rerender } = render(renderList("1"));
  const list = screen.getByLabelText("目录文件列表");
  measureList(list, 240);
  const pointer = { pointerId: 7, pointerType: "mouse", button: 0, buttons: 1 };
  const start = () => {
    fireEvent.pointerDown(list, { ...pointer, clientX: 1, clientY: 10 });
    fireEvent.pointerMove(list, { ...pointer, clientX: 95, clientY: 100 });
    expect(list.querySelector(".file-marquee")).not.toBeNull();
  };
  start();
  rerender(renderList("2"));
  expect(list.querySelector(".file-marquee")).toBeNull();
  fireEvent.pointerUp(list, { ...pointer, clientX: 95, clientY: 100 });
  expect(commit).not.toHaveBeenCalled();
  start();
  fireEvent.lostPointerCapture(list, pointer);
  expect(list.querySelector(".file-marquee")).toBeNull();
  expect(
    list.querySelector('[data-index="4"]')?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(commit).not.toHaveBeenCalled();
  start();
  expect(fireEvent.keyDown(list, { key: "Tab", ctrlKey: true })).toBe(true);
  expect(list.querySelector(".file-marquee")).toBeNull();
  expect(commit).not.toHaveBeenCalled();
});

test("scrolls a directory frame through the loaded edge and includes entries from the next page", () => {
  vi.useFakeTimers();
  try {
    const more = vi.fn();
    const commit = vi.fn();
    function Harness() {
      const [loaded, setLoaded] = useState(3);
      return (
        <FileList
          rows={files.slice(0, loaded)}
          selection={[]}
          onSelect={commit}
          onOpen={() => {}}
          next={loaded === 3 ? 3 : null}
          busy={false}
          onMore={() => {
            more();
            setLoaded(12);
          }}
        />
      );
    }
    render(<Harness />);
    const list = screen.getByLabelText("目录文件列表");
    measureList(list, 72);
    const pointer = {
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    };
    fireEvent.pointerDown(list, { ...pointer, clientX: 1, clientY: 8 });
    fireEvent.pointerMove(list, { ...pointer, clientX: 90, clientY: 71 });
    act(() => vi.advanceTimersByTime(120));
    expect(more).toHaveBeenCalledOnce();
    act(() => vi.advanceTimersByTime(260));
    expect(list.scrollTop).toBeGreaterThan(102);
    fireEvent.pointerUp(list, { ...pointer, clientX: 90, clientY: 71 });
    expect(commit.mock.lastCall?.[0]).toContain("4");
    expect(commit.mock.lastCall?.[0][0]).toBe("0");
    const stopped = list.scrollTop;
    act(() => vi.advanceTimersByTime(100));
    expect(list.scrollTop).toBe(stopped);
  } finally {
    vi.useRealTimers();
  }
});
test("retains full-directory selection and right-click does not collapse it", () => {
  const select = vi.fn();
  const all = vi.fn();
  const menu = vi.fn();
  const row = {
    entry: [2],
    parent: [1],
    object: [2],
    name: "sample.txt",
    path: "sample.txt",
    kind: "file",
    attributes: 0,
    size: "0",
    created: null,
    modified: null,
  };
  render(
    <FileList
      rows={[row]}
      selection={[]}
      allSelected
      onSelect={select}
      onSelectAll={all}
      onContext={menu}
      onOpen={() => {}}
      next={1}
      busy={false}
      onMore={() => {}}
    />,
  );
  const item = screen.getByText("sample.txt").closest("tr")!;
  expect(item.getAttribute("aria-selected")).toBe("true");
  fireEvent.contextMenu(item, { clientX: 20, clientY: 30 });
  expect(menu).toHaveBeenCalledOnce();
  expect(select).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByLabelText("目录文件列表"), {
    key: "a",
    ctrlKey: true,
  });
  expect(all).toHaveBeenCalledOnce();
});
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
  expect(select.mock.lastCall?.[0]).toEqual([rows[1000].entry.join(",")]);
  fireEvent.keyDown(list, { ctrlKey: true, key: "a" });
  expect(select.mock.lastCall?.[0]).toHaveLength(3000);
  expect(screen.getAllByRole("row").length).toBeLessThan(60);
});

test("fills a tall directory viewport and exposes absolute positions for its windowed rows", () => {
  const rows = Array.from({ length: 300 }, (_, index) => ({
    entry: [index % 256, Math.floor(index / 256)],
    parent: [0],
    object: [index % 256, Math.floor(index / 256)],
    name: `item-${index}.txt`,
    path: `item-${index}.txt`,
    kind: "file",
    attributes: 0,
    size: null,
    created: null,
    modified: null,
  }));
  render(
    <FileList
      rows={rows}
      selection={[]}
      onSelect={() => {}}
      onOpen={() => {}}
      next={null}
      busy={false}
      onMore={() => {}}
    />,
  );
  const list = screen.getByLabelText("目录文件列表");
  Object.defineProperty(list, "clientHeight", { value: 2040 });
  fireEvent.scroll(list, { target: { scrollTop: 3400 } });
  const visible = screen.getByRole("row", { name: /item-159.txt/ });
  expect(visible.getAttribute("aria-rowindex")).toBe("161");
  expect(
    screen.getByRole("table", { name: "文件" }).getAttribute("aria-rowcount"),
  ).toBe("301");
  expect(screen.getAllByRole("row").length).toBeLessThan(90);
});
