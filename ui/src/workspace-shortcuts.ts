import { fileCommands } from "./file-operations";

export const workspaceShortcuts = {
  browse: ["新建浏览标签页", "Ctrl+T"],
  search: ["新建搜索标签页", "Ctrl+Shift+F"],
  close: ["关闭标签页", "Ctrl+W"],
  focus: ["聚焦搜索框", "Ctrl+F"],
  refresh: ["刷新当前页", "F5"],
  settings: ["设置…", "Ctrl+P"],
  exit: ["退出", "Ctrl+Q"],
  help: ["搜索语法", "F1"],
  export: ["导出搜索结果…", "Ctrl+S"],
  nextTab: ["下一个标签页", "Ctrl+Tab"],
  previousTab: ["上一个标签页", "Ctrl+Shift+Tab"],
  preview: ["预览窗格", "Alt+P"],
  up: ["上一级目录", "Alt+↑"],
  back: ["后退", "Alt+←"],
  forward: ["前进", "Alt+→"],
  details: ["详情", "Ctrl+Shift+6"],
  small: ["缩略图（小图标）", "Ctrl+Shift+4"],
  medium: ["缩略图（中图标）", "Ctrl+Shift+3"],
  large: ["缩略图（大图标）", "Ctrl+Shift+2"],
  selectAll: ["全选", "Ctrl+A"],
  caseSensitive: ["区分大小写", "Ctrl+I"],
  wholeWord: ["完整单词", "Ctrl+B"],
  path: ["匹配路径", "Ctrl+U"],
  regex: ["使用正则表达式", "Ctrl+R"],
} as const;

export function workspaceCommand(
  id: keyof typeof workspaceShortcuts,
  run: () => void,
  disabled?: boolean,
) {
  const [label, shortcut] = workspaceShortcuts[id];
  return { label, shortcut, run, disabled };
}

export function eventShortcut(event: KeyboardEvent) {
  const key =
    /^Digit[0-9]$/.test(event.code) && event.ctrlKey && event.shiftKey
      ? event.code.slice(-1)
      : event.key === "ArrowUp" ||
          event.key === "ArrowLeft" ||
          event.key === "ArrowRight"
        ? ({ ArrowUp: "↑", ArrowLeft: "←", ArrowRight: "→" } as const)[
            event.key
          ]
        : event.key.length === 1
          ? event.key.toUpperCase()
          : event.key;
  return `${event.ctrlKey ? "Ctrl+" : ""}${event.altKey ? "Alt+" : ""}${event.shiftKey ? "Shift+" : ""}${key}`;
}

const shortcutGroups = [
  [
    "工作区",
    [
      "browse",
      "search",
      "close",
      "focus",
      "refresh",
      "settings",
      "exit",
      "help",
      "nextTab",
      "previousTab",
    ],
  ],
  [
    "视图与目录",
    ["preview", "up", "back", "forward", "details", "small", "medium", "large"],
  ],
  ["当前搜索", ["caseSensitive", "wholeWord", "path", "regex", "export"]],
] as const;

export const shortcutReference = [
  ...shortcutGroups.map(([label, ids]) => ({
    label,
    keys: ids.map((id) => workspaceShortcuts[id]),
  })),
  {
    label: "文件列表",
    keys: [
      workspaceShortcuts.selectAll,
      ...Object.values(fileCommands).filter(([, shortcut]) => shortcut),
      ["打开文件右键菜单", "Shift+F10"],
      ["自动调整列宽", "Ctrl+数字键盘加号"],
    ],
  },
];
