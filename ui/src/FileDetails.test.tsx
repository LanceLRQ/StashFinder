import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import FileDetails from "./FileDetails";

afterEach(cleanup);

const row = {
  entry: [1],
  parent: [0],
  object: [1],
  name: "sample.txt",
  path: "sample.txt",
  kind: "file",
  size: "23",
  attributes: 0,
  created: null,
  modified: null,
};

test("a partial loaded selection cannot impersonate a single file", () => {
  render(
    <FileDetails
      rows={[row]}
      selectionCount={2}
      partition=""
      connection={null}
      indexedAt={null}
    />,
  );
  expect(screen.getByText("已选择 2 项")).toBeTruthy();
  expect(screen.queryByText("sample.txt")).toBeNull();
  expect(screen.queryByText("大小")).toBeNull();
});

test("a single file still exposes uncertain status and missing metadata", () => {
  render(
    <FileDetails
      rows={[row]}
      partition="Movies"
      connection="unconfirmed"
      indexedAt={null}
    />,
  );
  expect(screen.getByText("状态未确认")).toBeTruthy();
  expect(screen.getByText("索引时间")).toBeTruthy();
  expect(screen.getAllByText("未知")).toHaveLength(3);
});

test("multiple selection omits unavailable aggregate properties", () => {
  render(
    <FileDetails
      rows={[]}
      selectionCount={2}
      partition=""
      connection={null}
      indexedAt={null}
    />,
  );
  const preview = within(screen.getByLabelText("预览窗格"));
  expect(preview.getByText("已选择 2 项")).toBeTruthy();
  expect(preview.queryByText("分区")).toBeNull();
  expect(preview.queryByText("状态未确认")).toBeNull();
  expect(preview.queryByText("索引时间")).toBeNull();
});

test("multiple selection retains known shared properties", () => {
  render(
    <FileDetails
      rows={[]}
      selectionCount={2}
      partition="Movies"
      connection="offline"
      indexedAt={1000}
    />,
  );
  expect(screen.getByText("Movies")).toBeTruthy();
  expect(screen.getByText("离线")).toBeTruthy();
  expect(screen.getByText("索引时间")).toBeTruthy();
});
