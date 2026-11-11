import {
  fileSize,
  fileTime,
  type DirectorySortField,
  type FileRow,
} from "./browse-api";

export type FileColumnId =
  | DirectorySortField
  | "partition"
  | "status"
  | "group";
export type FileColumn = {
  id: FileColumnId;
  label: string;
  width: number;
  sortable?: boolean;
  optional?: boolean;
};

const name: FileColumn = {
  id: "name",
  label: "名称",
  width: 260,
  sortable: true,
};
const path: FileColumn = {
  id: "path",
  label: "所在目录 / 路径",
  width: 340,
  sortable: true,
};
const metadata: FileColumn[] = [
  { id: "size", label: "大小", width: 100, sortable: true },
  {
    id: "extension",
    label: "扩展名",
    width: 92,
    sortable: true,
    optional: true,
  },
  { id: "type", label: "类型", width: 124, sortable: true, optional: true },
  { id: "modified", label: "修改时间", width: 165, sortable: true },
  {
    id: "created",
    label: "创建时间",
    width: 165,
    sortable: true,
    optional: true,
  },
  {
    id: "attributes",
    label: "属性",
    width: 105,
    sortable: true,
    optional: true,
  },
];

export const directoryColumns: readonly FileColumn[] = [
  name,
  { ...path, optional: true },
  ...metadata,
];
export const searchColumns: readonly FileColumn[] = [
  name,
  path,
  { id: "partition", label: "分区", width: 108, sortable: true },
  ...metadata,
  { id: "status", label: "状态", width: 150 },
  { id: "group", label: "重复组", width: 75, sortable: true },
];

const attributeFlags = [
  [1, "R"],
  [2, "H"],
  [4, "S"],
  [16, "D"],
  [32, "A"],
  [128, "N"],
  [256, "T"],
  [512, "P"],
  [1024, "L"],
  [2048, "C"],
  [4096, "O"],
  [8192, "I"],
  [16384, "E"],
] as const;

export function fileColumnText(file: FileRow, column: FileColumnId) {
  switch (column) {
    case "path":
      return file.path;
    case "size":
      return file.kind === "folder" ? "—" : fileSize(file.size);
    case "modified":
    case "created":
      return fileTime(file[column]);
    case "attributes":
      return (
        attributeFlags
          .filter(([flag]) => file.attributes & flag)
          .map(([, letter]) => letter)
          .join("") || "—"
      );
    case "extension":
    case "type": {
      const dot = file.name.lastIndexOf(".");
      const ext =
        file.kind === "file" && dot >= 0 ? file.name.slice(dot + 1) : "";
      return column === "extension"
        ? ext || "—"
        : file.kind === "folder"
          ? "文件夹"
          : ext
            ? `${ext.toUpperCase()} 文件`
            : "文件";
    }
    default:
      return "—";
  }
}
