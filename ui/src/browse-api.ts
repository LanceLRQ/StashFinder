import { invoke } from "@tauri-apps/api/core";
import type { SearchHit } from "./index-api";

export type FileRow = SearchHit["file"];
export type Ancestor = { entry: number[]; name: string };
export type DirectoryPage = {
  filesystem_id: string;
  generation: string;
  revision: string;
  location: number[];
  ancestors: Ancestor[];
  entries: FileRow[];
  next: number | null;
  partial: boolean;
  indexed_at: number | null;
  needs_validation: boolean;
};
export type DirectoryRequest = {
  partition_id: string;
  filesystem_id: string;
  parent: number[] | null;
  generation?: string | null;
  revision?: string | null;
  offset?: number;
  folders_only?: boolean;
};
export async function readDirectory(request: DirectoryRequest) {
  const reply = await invoke<
    { kind: "directory"; data: DirectoryPage } | { kind: "error"; data: string }
  >("index_request", {
    action: {
      directory: {
        generation: null,
        revision: null,
        offset: 0,
        folders_only: false,
        ...request,
      },
    },
  });
  if (reply.kind !== "directory") throw new Error(reply.data);
  return reply.data;
}

export function fileTime(value: string | null) {
  if (value == null) return "未知";
  return new Date(
    Number(BigInt(value) / 10000n - 11644473600000n),
  ).toLocaleString();
}
export const connectionLabel: Record<string, string> = {
  online: "在线",
  offline: "离线",
  uncertain: "身份待确认",
  changed: "历史记录",
  historical: "历史记录",
  inaccessible: "无法访问",
  unconfirmed: "状态未确认",
};
