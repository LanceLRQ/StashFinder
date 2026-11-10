import type { ComponentProps } from "react";
import DetailsViewport from "./DetailsViewport";

export const searchDetailsLayout = { rowHeight: 64, overscan: 8 };

const columns = [
  { label: "名称", sort: "name" },
  { label: "所在目录 / 路径", sort: "path" },
  { label: "分区", sort: "partition" },
  { label: "大小", sort: "size" },
  { label: "修改时间", sort: "modified" },
  { label: "状态 / 索引时间" },
  { label: "重复组", sort: "group" },
];

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
      columns={columns}
    />
  );
}
