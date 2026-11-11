import type { ComponentProps, ReactNode } from "react";
import DetailsViewport from "./DetailsViewport";
import { fileDetailsLayout } from "./file-view";
import { searchColumns } from "./file-columns";

export const searchDetailsLayout = { ...fileDetailsLayout, overscan: 8 };

export default function VirtualResults({
  overlay,
  ...props
}: Omit<
  ComponentProps<typeof DetailsViewport>,
  "layout" | "columns" | "label" | "className"
> & { overlay?: ReactNode }) {
  return (
    <div className="search-result-pane">
      <DetailsViewport
        {...props}
        className="virtual-results"
        label="搜索结果列表"
        layout={searchDetailsLayout}
        columns={searchColumns}
      />
      {overlay}
    </div>
  );
}
