import { workspaceCommand } from "./workspace-shortcuts";

export const fileDetailsLayout = { rowHeight: 34, overscan: 10 };

export const thumbnailLayouts = {
  small: { size: 32, tileWidth: 104, rowHeight: 82 },
  medium: { size: 64, tileWidth: 144, rowHeight: 108 },
  large: { size: 128, tileWidth: 208, rowHeight: 176 },
};

export type FileView = "details" | keyof typeof thumbnailLayouts;

export function fileViewOptions(
  current: FileView,
  change: (view: FileView) => void,
) {
  return (["large", "medium", "small", "details"] as const).map((view) => ({
    ...workspaceCommand(view, () => change(view)),
    checked: current === view,
    radio: true,
  }));
}
