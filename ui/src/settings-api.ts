import { invoke } from "@tauri-apps/api/core";
import type { Category, FilterPreset } from "./search-options";

export type Page = "overview" | "files" | "tasks" | "settings";
export type Preferences = {
  version: 1;
  collapsed: boolean;
  start_page: Page;
  include_path: boolean;
  categories?: Category[];
  filter_presets?: FilterPreset[];
  browse_filter?: string | null;
};
export const defaultPreferences: Preferences = {
  version: 1,
  collapsed: false,
  start_page: "overview",
  include_path: true,
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
