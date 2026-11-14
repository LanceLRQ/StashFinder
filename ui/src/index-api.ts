import { invoke } from "@tauri-apps/api/core";
import type { SearchOptions } from "./search-options";

export type IndexStatus = {
  task_id?: string | null;
  journal_missing?: boolean;
  generation: number | null;
  active_generation: number | null;
  phase: string;
  started: number | null;
  completed: number | null;
  records: number;
  error: string | null;
};
export type SearchHit = {
  duplicate_group?: number | null;
  hard_link?: boolean;
  partition_id: string;
  filesystem_id: string;
  partition_name: string;
  connection: string;
  mounts: string[];
  generation: number;
  revision: number;
  partial: boolean;
  indexed_at: number | null;
  needs_validation: boolean;
  file: {
    entry: number[];
    parent: number[];
    object: number[];
    name: string;
    path: string;
    kind: string;
    attributes: number;
    size: string | null;
    created: string | null;
    modified: string | null;
  };
};
export type SearchPage = {
  rows: SearchHit[];
  total: number;
  groups: number;
  missing: number;
  id: number;
  done: boolean;
  row: SearchHit | null;
  next: number | null;
  more: boolean;
  error: string | null;
  unavailable: string[];
};
type Action =
  | { index_state: { partition_id: string } }
  | {
      index_start: {
        partition_id: string;
        mode: "journal" | "create_journal" | "scan_only";
      };
    }
  | { task_cancel: { id: string } }
  | {
      search_start: {
        text: string;
        names_only: boolean;
        partitions: string[];
        filesystem_id?: string;
        replaces?: number;
        options?: SearchOptions;
      };
    }
  | { search_page: { id: number; offset: number } }
  | { search_reselect: { id: number; previous: number; offset: number } }
  | {
      search_batch: {
        id: number;
        offset: number;
        limit: number;
        wait_ms?: number;
      };
    }
  | { search_cancel: { id: number } };
type Reply =
  | { kind: "accepted" }
  | { kind: "index"; data: IndexStatus }
  | { kind: "search_started"; data: number }
  | { kind: "search_position"; data: number | null }
  | { kind: "search_page"; data: SearchPage };
export function indexRequest(action: Action) {
  return invoke<Reply>("index_request", { action });
}
