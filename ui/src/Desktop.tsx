import { lazy, Suspense, useEffect, useRef, useState } from "react";
import Assets, { type AssetControls } from "./Assets";
import Workspace from "./Workspace";
import FileOperations from "./FileOperations";
import Visuals from "./Visuals";
import Settings from "./Settings";
import type { Asset, AssetSnapshot } from "./asset-api";
import Tasks from "./Tasks";
import { IndexModeDialog } from "./IndexControls";
import { useTasks } from "./task-api";
import {
  defaultPreferences,
  readPreferences,
  writePreferences,
  type Preferences,
} from "./settings-api";
import "./desktop.css";

const Development = import.meta.env.DEV ? lazy(() => import("./App")) : null;
const noAssets: Asset[] = [];

export default function Desktop() {
  const [preferences, setPreferences] = useState(defaultPreferences);
  const [loaded, setLoaded] = useState(false);
  const [preferenceError, setPreferenceError] = useState("");
  const [saving, setSaving] = useState(false);
  const saveLock = useRef(false);
  const [snapshot, setSnapshot] = useState<AssetSnapshot | null>(null);
  const [deviceStatus, setDeviceStatus] = useState({
    error: "",
    working: false,
  });
  const deviceControls = useRef<AssetControls>(null);
  const [searchRequest, setSearchRequest] = useState({ text: "", revision: 0 });
  const [browseRequest, setBrowseRequest] = useState<{
    partition: string;
    revision: number;
  }>();
  const tasks = useTasks(true);
  const [indexChoice, setIndexChoice] = useState("");
  const assets = snapshot?.assets ?? noAssets;
  const partitions = assets.map((a) => ({
    id: a.registration.partition_id,
    name: a.registration.name,
    connection: a.connection,
    canChooseIndex:
      !snapshot?.error &&
      !snapshot?.busy &&
      !a.error &&
      a.connection === "online" &&
      a.registration.state === "ready" &&
      a.registration.observation.identity.filesystem === "NTFS",
  }));
  const choicePartition = partitions.find((p) => p.id === indexChoice);
  useEffect(() => {
    let active = true;
    void readPreferences()
      .then(
        (value) => {
          if (active) setPreferences(value);
        },
        (error) => {
          if (active) setPreferenceError(`设置读取失败：${String(error)}`);
        },
      )
      .then(() => {
        if (active) setLoaded(true);
      });
    return () => {
      active = false;
    };
  }, []);
  async function save(next: Preferences) {
    if (saveLock.current) return;
    saveLock.current = true;
    setSaving(true);
    await writePreferences(next).then(
      (value) => {
        setPreferences(value);
        setPreferenceError("");
      },
      (e) => setPreferenceError(`设置未保存：${String(e)}`),
    );
    setSaving(false);
    saveLock.current = false;
  }

  return (
    <Visuals>
      <FileOperations>
        <main className="desktop">
          <Workspace
            assets={assets}
            includePath={preferences.include_path}
            categories={preferences.categories}
            request={searchRequest}
            searchReady={loaded}
            browseRequest={browseRequest}
            completedTasks={tasks.data?.tasks.filter(
              (t) => t.state === "completed",
            )}
            automatic={tasks.data?.automatic}
            paused={tasks.data?.paused}
            startupReady={
              !!snapshot &&
              !snapshot.error &&
              !deviceStatus.error &&
              (!snapshot.busy || assets.length > 0)
            }
            device={{
              ...deviceStatus,
              busy: !!snapshot?.busy || deviceStatus.working,
              register: () => deviceControls.current?.register(),
              refresh: () => deviceControls.current?.refresh(),
              retry: () => deviceControls.current?.retry(),
            }}
            notice={
              preferenceError && (
                <p role="alert" className="desktop-error preference-error">
                  {preferenceError}
                </p>
              )
            }
            panels={{
              overview: (
                <Assets
                  ref={deviceControls}
                  onSearch={(text) =>
                    setSearchRequest((previous) => ({
                      text,
                      revision: previous.revision + 1,
                    }))
                  }
                  onBrowse={(partition) =>
                    setBrowseRequest((previous) => ({
                      partition,
                      revision: (previous?.revision ?? 0) + 1,
                    }))
                  }
                  onSnapshot={setSnapshot}
                  onStatus={setDeviceStatus}
                />
              ),
              tasks: (
                <Tasks
                  data={tasks.data}
                  error={tasks.error}
                  onRefresh={tasks.refresh}
                  onChooseIndex={setIndexChoice}
                  partitions={partitions}
                />
              ),
              settings: (
                <Settings
                  preferences={preferences}
                  saving={saving || !loaded}
                  onSave={save}
                  tasks={tasks.data}
                  taskError={tasks.error}
                  onTasksRefresh={tasks.refresh}
                />
              ),
              ...(Development && {
                development: (
                  <Suspense fallback={<p>正在加载验证工具…</p>}>
                    <Development />
                  </Suspense>
                ),
              }),
            }}
          />
          {choicePartition && (
            <IndexModeDialog
              partitionId={indexChoice}
              name={choicePartition.name}
              eligible={!tasks.error && choicePartition.canChooseIndex}
              ntfs
              journalMissing
              onClose={() => setIndexChoice("")}
              onStarted={() => {
                setIndexChoice("");
                tasks.refresh();
              }}
            />
          )}
        </main>
      </FileOperations>
    </Visuals>
  );
}
