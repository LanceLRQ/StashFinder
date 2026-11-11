export type Observation = {
  candidate_id: string;
  disk_number: number | null;
  model: string | null;
  disk_bytes: number | null;
  partition_bytes: number | null;
  volume_label?: string | null;
  mounts: string[];
  identity: {
    layout: Record<string, unknown>;
    filesystem: string | null;
    serial: { source: string; value: string } | null;
  };
  capacity: { total: number; free: number; sampled_at: number } | null;
  error: string | null;
};
export type Registration = {
  disk_id: string;
  partition_id: string;
  filesystem_id: string;
  name: string;
  revision: number;
  state: string;
  observation: Observation;
  last_seen: number | null;
};
export type Asset = {
  capacity_error?: string | null;
  registration: Registration;
  connection: string;
  mounts: string[];
  error: string | null;
};
export type Candidate = {
  observation: Observation;
  weak: boolean;
  blocked: string | null;
  registered?: boolean;
};
export type AssetSnapshot = {
  version: number;
  busy: boolean;
  error: string | null;
  assets: Asset[];
  candidates: Candidate[];
};
export type AssetAction =
  | "refresh"
  | { rename: { partition_id: string; name: string } }
  | { unregister: { partition_id: string } }
  | { retry: { version: number; partition_id: string; candidate_id: string } };
