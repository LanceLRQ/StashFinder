import { useLayoutEffect, useRef } from "react";
import type { MenuCommand } from "./AppMenu";

export const maxFileSelection = 10_000;
export const fileSelectionLimit = "文件操作最多支持 10,000 项";

export type EntryRef = {
  partition_id: string;
  filesystem_id: string;
  generation: string;
  revision: string;
  entry: number[];
};
export type SelectionOrigin =
  | {
      kind: "directory";
      context: EntryRef;
      all: boolean;
      filter?: import("./search-options").FileFilter;
    }
  | { kind: "search"; id: number; range: [number, number] | null };
export type SelectionInput = {
  origin: SelectionOrigin;
  entries: number[][];
  positions: number[];
};
export type FileAction =
  | "copy"
  | "cut"
  | "drag"
  | "open"
  | "open_with"
  | "locate"
  | "properties"
  | "recycle"
  | "delete"
  | "copy_name"
  | "copy_path"
  | { rename: string }
  | { more: { x: number; y: number } };
export type PreparedSummary = {
  id: number;
  count: number;
  names: string[];
  same_parent: boolean;
  single_file: boolean;
};
export type OperationStatus = {
  close_requested?: boolean;
  id: number;
  phase: string;
  total: number;
  processed: number;
  error: string | null;
};
export type ReportPage = {
  id: number;
  total: number;
  completed: number;
  rows: {
    index: number;
    source: string;
    state: "not_run" | "completed" | "skipped" | "failed";
    destination: string | null;
    error: string | null;
    note: string | null;
  }[];
  next: number | null;
};
export const fileCommands = {
  open: ["打开", "Enter"],
  open_with: ["打开方式...", ""],
  browse_parent: ["在新标签页浏览所在目录", ""],
  locate: ["在资源管理器显示", ""],
  cut: ["剪切", "Ctrl+X"],
  copy: ["复制文件", "Ctrl+C"],
  paste: ["粘贴", "Ctrl+V"],
  copy_name: ["复制名称", ""],
  copy_path: ["复制路径", "Ctrl+Shift+C"],
  rename: ["重命名", "F2"],
  recycle: ["删除", "Delete"],
  properties: ["属性", "Alt+Enter"],
  more: ["更多...", ""],
  delete: ["永久删除", "Shift+Delete"],
} as const;
export type MenuAction = keyof typeof fileCommands;
export const editCommands: MenuAction[] = [
  "cut",
  "copy",
  "paste",
  "copy_name",
  "copy_path",
  "rename",
  "recycle",
  "properties",
];
export const contextCommands = (
  Object.keys(fileCommands) as MenuAction[]
).filter((action) => action !== "delete");
export type CommandFacts = {
  count: number;
  online: boolean;
  singleFile: boolean;
  sameParent: boolean;
  folder?: boolean;
  paste?: boolean;
  blocked?: boolean;
};
export function commandEnabled(action: MenuAction, facts: CommandFacts) {
  if (facts.blocked) return false;
  const { count, online, singleFile, sameParent, folder, paste } = facts;
  if (count > maxFileSelection && action !== "paste" && action !== "properties")
    return false;
  switch (action) {
    case "open":
      return count === 1 && (online || !!folder);
    case "open_with":
      return online && singleFile;
    case "browse_parent":
      return count === 1;
    case "locate":
    case "rename":
      return online && count === 1;
    case "cut":
    case "copy":
    case "recycle":
    case "delete":
      return online && count > 0;
    case "paste":
      return !!paste;
    case "copy_name":
    case "copy_path":
    case "properties":
      return count > 0;
    case "more":
      return online && count > 0 && sameParent;
  }
}
export type FileMenuActions = {
  export?: MenuCommand[];
  selection?: MenuCommand[];
  sorting?: MenuCommand[];
  navigation?: MenuCommand[];
  search?: MenuCommand[];
  presets?: MenuCommand[];
};
export type FileCommandContext = {
  enabled: (action: MenuAction) => boolean;
  dispatch: (action: MenuAction) => void;
  views: MenuCommand[];
  actions?: FileMenuActions;
  refresh?: () => void;
};
export function useFileCommands(
  facts: CommandFacts,
  run: (action: MenuAction) => void,
  active: boolean,
  onContext: ((context: FileCommandContext | null) => void) | undefined,
  views: MenuCommand[],
  refresh?: () => void,
  actions: FileMenuActions = {},
) {
  const current = useRef({ facts, run, active, views, refresh, actions });
  useLayoutEffect(() => {
    current.current = { facts, run, active, views, refresh, actions };
  });
  const enabled = (action: MenuAction) =>
    current.current.active && commandEnabled(action, current.current.facts);
  const dispatch = (action: MenuAction) => {
    if (enabled(action)) current.current.run(action);
  };
  const availability = editCommands
    .map((id) => Number(commandEnabled(id, facts)))
    .join("");
  const menuState = JSON.stringify({ views, actions });
  const canRefresh = refresh !== undefined;
  useLayoutEffect(() => {
    if (!active || !onContext) return;
    onContext({
      enabled,
      dispatch,
      refresh: canRefresh
        ? () => {
            if (current.current.active) current.current.refresh?.();
          }
        : undefined,
      views: current.current.views.map((view, index) => ({
        ...view,
        run: () => current.current.views[index]?.run(),
      })),
      actions: Object.fromEntries(
        (Object.keys(actions) as (keyof FileMenuActions)[]).map((group) => [
          group,
          actions[group]?.map((item, index) => ({
            ...item,
            run: () => {
              const command = current.current.actions[group]?.[index];
              if (current.current.active && command && !command.disabled)
                command.run();
            },
          })),
        ]),
      ),
    });
    return () => onContext(null);
  }, [active, onContext, availability, menuState, canRefresh]);
  return dispatch;
}

export function editableTarget(target: EventTarget | null) {
  return (
    target instanceof Element &&
    !!target.closest(
      'input,textarea,select,[contenteditable]:not([contenteditable="false"])',
    )
  );
}

export function fileKey(
  e: React.KeyboardEvent | KeyboardEvent,
  dispatch: (action: MenuAction) => void,
  menu?: () => void,
) {
  if (e.metaKey || fileEditingKey(e)) return false;
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  const shortcut = [
    e.ctrlKey ? "Ctrl+" : "",
    e.altKey ? "Alt+" : "",
    e.shiftKey ? "Shift+" : "",
    key,
  ].join("");
  const action = (Object.keys(fileCommands) as MenuAction[]).find(
    (id) => id !== "open" && fileCommands[id][1] === shortcut,
  );
  const context =
    menu && (shortcut === "ContextMenu" || shortcut === "Shift+F10");
  if (!action && !context) return false;
  e.preventDefault();
  e.stopPropagation();
  if (action) dispatch(action);
  else menu?.();
  return true;
}

export function fileEditingKey(e: React.KeyboardEvent | KeyboardEvent) {
  return (
    e.defaultPrevented ||
    ("nativeEvent" in e ? e.nativeEvent.isComposing : e.isComposing) ||
    e.key === "Process" ||
    e.keyCode === 229 ||
    editableTarget(e.target)
  );
}
