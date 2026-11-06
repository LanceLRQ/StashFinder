export type VisualImage = {
  data_url: string | null;
  source: string;
  historical: boolean;
  warning: string | null;
};
export type CacheStatus = {
  used_bytes: number;
  limit_bytes: number;
  files: number;
  waiting: number;
  active: number;
};
export type CacheProgress = {
  processed: number;
  total: number;
  used_bytes: number;
  limit_bytes: number;
};
export const visualPhases: Record<string, string> = {
  waiting: "等待系统处理…",
  record: "读取文件记录…",
  local_cache: "读取本地缓存…",
  system_cache: "读取系统缩略图…",
  system_generate: "Windows 正在生成缩略图…",
};
export function cacheBytes(bytes: number) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let unit = 0;
  while (bytes >= 1000 && unit < units.length - 1) {
    bytes /= 1000;
    unit++;
  }
  return `${bytes.toLocaleString(undefined, { maximumFractionDigits: 1 })} ${units[unit]}`;
}
