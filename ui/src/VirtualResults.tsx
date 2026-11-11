import type { ComponentProps } from "react";
import DetailsViewport from "./DetailsViewport";
import { fileDetailsLayout } from "./file-view";
import { searchColumns } from "./file-columns";

export const searchDetailsLayout = { ...fileDetailsLayout, overscan: 8 };

export default function VirtualResults(
  props: Omit<
    ComponentProps<typeof DetailsViewport>,
    "layout" | "columns" | "label" | "className"
  >,
) {
  return (
    <DetailsViewport
      {...props}
      className="virtual-results"
      label="搜索结果列表"
      layout={searchDetailsLayout}
      columns={searchColumns}
    />
  );
}
