import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import RenameInput from "./RenameInput";
afterEach(cleanup);
test("selects the stem and submits with Enter without opening the file", () => {
  const submit = vi.fn();
  render(
    <RenameInput
      name="sample.txt"
      folder={false}
      onSubmit={submit}
      onCancel={vi.fn()}
    />,
  );
  const input = screen.getByRole("textbox") as HTMLInputElement;
  expect(input.selectionStart).toBe(0);
  expect(input.selectionEnd).toBe(6);
  fireEvent.change(input, { target: { value: "renamed.txt" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(submit).toHaveBeenCalledWith("renamed.txt");
});
