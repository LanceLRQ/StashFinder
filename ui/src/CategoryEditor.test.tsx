import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import CategoryEditor from "./CategoryEditor";
afterEach(cleanup);
test("normalizes custom extensions and leaves the old list until saving succeeds", async () => {
  const save = vi.fn().mockResolvedValue(undefined);
  const view = render(
    <CategoryEditor
      categories={[{ name: "视频", extensions: ["mkv"] }]}
      saving={false}
      onSave={save}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "新增类型" }));
  fireEvent.change(screen.getByLabelText("分类名称"), {
    target: { value: "照片" },
  });
  fireEvent.change(screen.getByLabelText("分类扩展名"), {
    target: { value: ".JPG,jpg,PNG" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存类型" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith([
      { name: "视频", extensions: ["mkv"] },
      { name: "照片", extensions: ["jpg", "png"] },
    ]),
  );
  expect(screen.getByLabelText("分类名称")).toHaveProperty("value", "照片");
  view.rerender(
    <CategoryEditor
      categories={[{ name: "照片", extensions: ["jpg", "png"] }]}
      saving={false}
      onSave={save}
    />,
  );
  expect(screen.queryByLabelText("分类名称")).toBeNull();
});
