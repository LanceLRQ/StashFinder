export const thumbnailLayouts = {
  small: { size: 32, tileWidth: 104, rowHeight: 82 },
  medium: { size: 64, tileWidth: 144, rowHeight: 108 },
  large: { size: 128, tileWidth: 208, rowHeight: 176 },
};

export type FileView = "details" | keyof typeof thumbnailLayouts;

const labels = {
  details: "详情",
  small: "缩略图（小图标）",
  medium: "缩略图（中图标）",
  large: "缩略图（大图标）",
} satisfies Record<FileView, string>;

export function fileViewOptions(
  current: FileView,
  change: (view: FileView) => void,
) {
  return (Object.keys(labels) as FileView[]).map((view) => ({
    label: labels[view],
    checked: current === view,
    radio: true,
    run: () => change(view),
  }));
}
