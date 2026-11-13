export type Category = { name: string; extensions: string[] };
export const defaultCategories: Category[] = [
  {
    name: "视频",
    extensions: ["mp4", "mkv", "avi", "mov", "wmv", "webm", "ts", "m2ts"],
  },
  {
    name: "图片",
    extensions: [
      "jpg",
      "jpeg",
      "png",
      "gif",
      "webp",
      "bmp",
      "tif",
      "tiff",
      "heic",
      "svg",
    ],
  },
  {
    name: "音频",
    extensions: ["mp3", "flac", "wav", "aac", "ogg", "m4a", "ape"],
  },
  {
    name: "文档",
    extensions: [
      "pdf",
      "doc",
      "docx",
      "xls",
      "xlsx",
      "ppt",
      "pptx",
      "txt",
      "md",
      "epub",
    ],
  },
  {
    name: "压缩包",
    extensions: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz"],
  },
  { name: "程序", extensions: ["exe", "msi", "dll", "bat", "cmd", "ps1"] },
];
export type Range = { min: string | null; max: string | null };
export type FileFilter = {
  extensions: string[];
  include_folders: boolean;
  kind: "file" | "folder" | null;
  size: Range;
  created: Range;
  modified: Range;
};
export type FilterPreset = { id: string; name: string; rule: FileFilter };
export type FilterSelection = {
  rule: FileFilter;
  types: string[];
  custom: string;
  id?: string;
};
export const noPresets: FilterPreset[] = [];
export function filterPresetEntries(
  categories: Category[],
  presets: FilterPreset[],
) {
  return [
    { id: null, name: "无筛选" },
    { id: "@folder", name: "文件夹" },
    ...categories.map(({ name }) => ({ id: `category:${name}`, name })),
    ...presets.map(({ id, name }) => ({ id, name: `个人：${name}` })),
  ];
}
export const emptyFileFilter: FileFilter = {
  extensions: [],
  include_folders: false,
  kind: null,
  size: { min: null, max: null },
  created: { min: null, max: null },
  modified: { min: null, max: null },
};
export const noFilter: FilterSelection = {
  rule: emptyFileFilter,
  types: [],
  custom: "",
};
export const hasFileFilter = (rule: FileFilter) =>
  rule.extensions.length > 0 ||
  rule.kind !== null ||
  [rule.size, rule.created, rule.modified].some((r) => r.min || r.max);

export function selectFilter(
  id: string | null,
  categories: Category[],
  presets: FilterPreset[],
): FilterSelection {
  const preset = presets.find((p) => p.id === id);
  if (preset) {
    return {
      id: preset.id,
      rule: preset.rule,
      types:
        preset.rule.include_folders || preset.rule.kind === "folder"
          ? ["@folder"]
          : [],
      custom: preset.rule.extensions.join(","),
    };
  }
  if (id === "@folder") {
    return {
      ...noFilter,
      types: [id],
      rule: { ...emptyFileFilter, kind: "folder" },
    };
  }
  const category = categories.find((c) => `category:${c.name}` === id);
  return category
    ? {
        ...noFilter,
        types: [`category:${category.name}`],
        rule: { ...emptyFileFilter, extensions: category.extensions },
      }
    : noFilter;
}
export type SearchContext = {
  partition: string;
  entry: number[];
  name: string;
};
export type SearchOptions = FileFilter & {
  case_sensitive: boolean;
  mode: string;
  online_only: boolean;
  include_history: boolean;
  duplicate: string[];
  sort: string;
  descending: boolean;
  scope: { partition_id: string; entry: number[]; recursive: boolean } | null;
};
export const emptyOptions: SearchOptions = {
  ...emptyFileFilter,
  case_sensitive: false,
  mode: "contains",
  online_only: false,
  include_history: false,
  duplicate: [],
  sort: "name",
  descending: false,
  scope: null,
};
export function dateBoundary(day: string, end: boolean) {
  if (!day) return null;
  const date = new Date(`${day}T00:00:00`);
  if (end) date.setDate(date.getDate() + 1);
  return (
    (BigInt(date.getTime()) + 11644473600000n) * 10000n -
    (end ? 1n : 0n)
  ).toString();
}
export function dateLabel(value: string | null) {
  if (value === null) return "";
  const d = new Date(Number(BigInt(value) / 10000n - 11644473600000n));
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
