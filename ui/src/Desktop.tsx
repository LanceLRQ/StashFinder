import { lazy, Suspense, useEffect, useRef, useState } from "react";
import Assets, { type AssetControls } from "./Assets";
import Workspace from "./Workspace";
import FileOperations from "./FileOperations";
import Visuals from "./Visuals";
import Settings from "./Settings";
import type { Asset, AssetSnapshot } from "./asset-api";
import IndexActivity from "./IndexActivity";
import { IndexModeDialog } from "./IndexControls";
import { taskGroup, useTasks } from "./task-api";
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
  const partitions = assets.map((a) => {
    const canRetry =
      !snapshot?.error &&
      !snapshot?.busy &&
      !a.error &&
      a.connection === "online" &&
      a.registration.state === "ready";
    return {
      id: a.registration.partition_id,
      name: a.registration.name,
      canRetry,
      canChooseIndex:
        canRetry && a.registration.observation.identity.filesystem === "NTFS",
    };
  });
  const choicePartition = partitions.find((p) => p.id === indexChoice);
  const rebuildChoice = tasks.data?.tasks.some(
    (task) =>
      task.partition_id === indexChoice &&
      task.kind === "rebuild" &&
      task.state === "needs_confirmation",
  );
  const attentionPartitions = new Set<string>();
  const managedIndexTasks = new Set<string>();
  for (const task of tasks.data?.tasks ?? []) {
    const group = taskGroup(task);
    if (group === "attention") attentionPartitions.add(task.partition_id);
    if (
      !tasks.error &&
      (group === "current" || task.state === "needs_confirmation")
    )
      managedIndexTasks.add(task.partition_id);
  }
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
            indexes={tasks.data?.indexes}
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
              refresh: () => {
                deviceControls.current?.refresh();
                tasks.refresh();
              },
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
                  attentionPartitions={attentionPartitions}
                  managedIndexTasks={managedIndexTasks}
                  indexActivity={(partition, onSelect) => (
                    <IndexActivity
                      data={tasks.data}
                      error={tasks.error}
                      partitions={partitions}
                      partition={partition}
                      onSelect={onSelect}
                      onRefresh={tasks.refresh}
                      onChooseIndex={setIndexChoice}
                    />
                  )}
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
              journalMissing={!rebuildChoice}
              rebuild={rebuildChoice}
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
