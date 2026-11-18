import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import Tasks from "./Tasks";

afterEach(cleanup);

test("shows unknown counters and an in-place retry for an initial task error", () => {
  const refresh = vi.fn();
  render(
    <Tasks
      data={null}
      error="后台不可用"
      partitions={[]}
      onRefresh={refresh}
      onChooseIndex={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: /最近动态.*—/ })).toBeTruthy();
  expect(screen.queryByText("正在读取任务…")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(refresh).toHaveBeenCalled();
});
