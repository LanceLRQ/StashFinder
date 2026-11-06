export type EntryRef = {
  partition_id: string;
  filesystem_id: string;
  generation: string;
  revision: string;
  entry: number[];
};
export type SelectionOrigin =
  | { kind: "directory"; context: EntryRef; all: boolean }
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
export function fileKey(
  e: React.KeyboardEvent,
  actions: {
    copy: () => void;
    cut: () => void;
    paste?: () => void;
    rename: () => void;
    remove: (permanent: boolean) => void;
    properties: () => void;
    menu: () => void;
  },
) {
  const target = e.target as HTMLElement;
  if (
    e.nativeEvent.isComposing ||
    target.closest('input,textarea,select,[contenteditable="true"]')
  )
    return false;
  const key = e.key.toLowerCase();
  let action: (() => void) | undefined;
  if (e.ctrlKey && !e.altKey) {
    if (key === "c") action = actions.copy;
    if (key === "x") action = actions.cut;
    if (key === "v") action = actions.paste;
  } else if (e.key === "F2") action = actions.rename;
  else if (e.key === "Delete") action = () => actions.remove(e.shiftKey);
  else if (e.altKey && e.key === "Enter") action = actions.properties;
  else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10"))
    action = actions.menu;
  if (action) {
    e.preventDefault();
    e.stopPropagation();
    action();
    return true;
  }
  return false;
}
