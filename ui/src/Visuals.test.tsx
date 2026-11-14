import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Visuals from "./Visuals";
import FileIcon from "./FileIcon";
import FileDetails from "./FileDetails";
import FileList from "./FileList";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.mocked(invoke).mockReset();
});
const png = "data:image/png;base64,aW1hZ2U=";
const reference = {
  partition_id: "p",
  filesystem_id: "fs",
  generation: "1",
  revision: "1",
  entry: [2],
};
const row = (name: string, id: number) => ({
  entry: [id],
  object: [id],
  parent: [1],
  name,
  path: name,
  kind: "file",
  size: "10",
  created: null,
  modified: null,
  attributes: 0,
});
const image = (url = png) => ({
  data_url: url,
  source: "system_icon",
  historical: false,
  warning: null,
});

test("thumbnail grids reuse versioned cache requests, cancel the old size and fall back offline", async () => {
  let resolveOld: (value: unknown) => void = () => {};
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command !== "visual_preview") return image();
    if (args.size === 32 && args.reference.entry[0] === 2)
      return new Promise((resolve) => {
        resolveOld = resolve;
      });
    if (args.reference.entry[0] === 3) throw Error("缩略图缓存未命中");
    return {
      ...image(png + "medium"),
      source: "local_cache",
      historical: true,
    };
  });
  const list = (view: "small" | "medium", active = true) => (
    <Visuals>
      <FileList
        view={view}
        rows={[row("hit.png", 2), row("miss.png", 3)]}
        context={reference}
        connection="offline"
        active={active}
        selection={[]}
        onSelect={() => {}}
        onOpen={() => {}}
        next={null}
        busy={false}
        onMore={() => {}}
      />
    </Visuals>
  );
  const view = render(list("small"));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("visual_preview", {
      requestId: expect.any(String),
      reference,
      size: 32,
      kind: "thumbnail",
    }),
  );
  view.rerender(list("medium"));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("visual_cancel", {
      requestId: expect.any(String),
    }),
  );
  await waitFor(() => {
    expect(
      screen
        .getByRole("option", { name: "hit.png" })
        .querySelector("img")
        ?.getAttribute("src"),
    ).toBe(png + "medium");
    expect(
      screen
        .getByRole("option", { name: "miss.png" })
        .querySelector("img")
        ?.getAttribute("src"),
    ).toBe(png);
  });
  await act(async () => resolveOld(image(png + "old")));
  expect(
    screen
      .getByRole("option", { name: "hit.png" })
      .querySelector("img")
      ?.getAttribute("src"),
  ).toBe(png + "medium");
  view.rerender(list("medium", false));
  await act(async () => {});
  expect(
    vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === "visual_preview"),
  ).toHaveLength(4);
});

test("visible files share a system type-icon request without requiring source paths", async () => {
  vi.mocked(invoke).mockResolvedValue(image());
  const view = render(
    <Visuals>
      <FileIcon name="first.txt" folder={false} />
      <FileIcon name="second.TXT" folder={false} />
    </Visuals>,
  );
  await waitFor(() =>
    expect(view.container.querySelectorAll("img")).toHaveLength(2),
  );
  expect(invoke).toHaveBeenCalledOnce();
  expect(invoke).toHaveBeenCalledWith("visual_icon", {
    name: "first.txt",
    folder: false,
    size: 32,
  });
});

test("inactive file views do not request background images", async () => {
  vi.mocked(invoke).mockResolvedValue(image());
  const view = render(
    <Visuals>
      <FileIcon name="hidden.txt" folder={false} active={false} />
    </Visuals>,
  );
  await act(async () => {});
  expect(invoke).not.toHaveBeenCalled();
  view.rerender(
    <Visuals>
      <FileIcon name="hidden.txt" folder={false} active />
    </Visuals>,
  );
  await waitFor(() => expect(invoke).toHaveBeenCalledOnce());
});

test("a late thumbnail cannot replace the new selection and obsolete work is cancelled", async () => {
  const pending: { requestId: string; resolve: (value: unknown) => void }[] =
    [];
  vi.mocked(invoke).mockImplementation((command, args: any) => {
    if (command === "visual_preview")
      return new Promise((resolve) =>
        pending.push({ requestId: args.requestId, resolve }),
      );
    return Promise.resolve(image());
  });
  const view = render(
    <Visuals>
      <FileDetails
        rows={[row("first.png", 2)]}
        partition="Photos"
        connection="online"
        indexedAt={null}
        reference={reference}
      />
    </Visuals>,
  );
  await waitFor(() => expect(pending).toHaveLength(1));
  view.rerender(
    <Visuals>
      <FileDetails
        rows={[row("second.png", 3)]}
        partition="Photos"
        connection="online"
        indexedAt={null}
        reference={{ ...reference, entry: [3] }}
      />
    </Visuals>,
  );
  await waitFor(() => expect(pending).toHaveLength(2));
  expect(invoke).toHaveBeenCalledWith("visual_cancel", {
    requestId: pending[0].requestId,
  });
  await act(async () => pending[1].resolve(image(png + "new")));
  expect(
    screen
      .getByRole("img", { name: "second.png 的缩略图" })
      .getAttribute("src"),
  ).toBe(png + "new");
  await act(async () => pending[0].resolve(image(png + "old")));
  expect(
    screen
      .getByRole("img", { name: "second.png 的缩略图" })
      .getAttribute("src"),
  ).toBe(png + "new");
  expect(listen).toHaveBeenCalledWith("visual-progress", expect.any(Function));
});

test("falls back to the type icon when the preview's index revision expires", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "visual_preview") throw Error("索引记录已变化，请刷新重选");
    return image();
  });
  const view = render(
    <Visuals>
      <FileDetails
        rows={[row("temporary.mp4", 2)]}
        partition="Movies"
        connection="online"
        indexedAt={null}
        reference={reference}
      />
    </Visuals>,
  );
  await act(async () => {});
  expect(
    view.container.querySelector(".thumbnail-image img")?.getAttribute("src"),
  ).toBe(png);
  expect(screen.getByText("10 B")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("offline cached thumbnails are marked historical while native failures leave metadata readable", async () => {
  vi.mocked(invoke).mockResolvedValue({
    ...image(),
    source: "local_cache",
    historical: true,
  });
  const view = render(
    <Visuals>
      <FileDetails
        rows={[row("photo.png", 2)]}
        partition="Photos"
        connection="offline"
        indexedAt={null}
        reference={reference}
      />
    </Visuals>,
  );
  expect(
    await screen.findByRole("img", { name: "photo.png 的缩略图" }),
  ).toBeTruthy();
  expect(screen.getByText("历史缓存")).toBeTruthy();
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "visual_preview") throw Error("系统缩略图读取失败");
    return image();
  });
  view.rerender(
    <Visuals>
      <FileDetails
        rows={[row("other.png", 3)]}
        partition="Photos"
        connection="online"
        indexedAt={null}
        reference={{ ...reference, entry: [3] }}
      />
    </Visuals>,
  );
  expect((await screen.findByRole("alert")).textContent).toBe(
    "系统缩略图读取失败",
  );
  expect(screen.getByText("Photos")).toBeTruthy();
  expect(screen.getByText("10 B")).toBeTruthy();
});
