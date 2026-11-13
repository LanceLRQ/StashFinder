import { invoke } from "@tauri-apps/api/core";
import type { Category, FilterPreset, SearchBookmark } from "./search-options";
import type { FileView } from "./file-view";

export type Page = "overview" | "files" | "tasks" | "settings";
export type Preferences = {
  version: 1;
  collapsed: boolean;
  start_page: Page;
  include_path: boolean;
  search_online_only?: boolean;
  search_include_history?: boolean;
  default_view?: FileView;
  default_preview?: boolean;
  show_status_bar?: boolean;
  categories?: Category[];
  filter_presets?: FilterPreset[];
  search_bookmarks?: SearchBookmark[];
  browse_filter?: string | null;
};
export const defaultPreferences: Preferences = {
  version: 1,
  collapsed: false,
  start_page: "overview",
  include_path: true,
  search_online_only: false,
  search_include_history: false,
  default_view: "details",
  default_preview: false,
  show_status_bar: true,
};
export type ServiceStatus = {
  version: number;
  id: number;
  process_id: number;
  mode: string;
  registered_partitions: number;
};
export function readPreferences() {
  return invoke<Preferences>("preferences_get");
}
export function writePreferences(preferences: Preferences) {
  return invoke<Preferences>("preferences_set", { preferences });
}
