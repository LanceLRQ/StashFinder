import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Assets, { type AssetControls } from "./Assets";
import Workspace from "./Workspace";
import FileOperations from "./FileOperations";
import Visuals from "./Visuals";
import Settings, { type SettingsPage } from "./Settings";
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
  const [preferenceUnavailable, setPreferenceUnavailable] = useState(false);
  const [settings, setSettings] = useState<SettingsPage | null>(null);
  const [exitError, setExitError] = useState("");
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
  const indexActivityLabels = new Map<string, string>();
  const managedIndexTasks = new Set<string>();
  for (const task of tasks.data?.tasks ?? []) {
    const group = taskGroup(task);
    if (group === "attention") attentionPartitions.add(task.partition_id);
    if (!tasks.error && group === "current") {
      indexActivityLabels.set(
        task.partition_id,
        task.state === "running"
          ? task.kind === "sync"
            ? "同步中"
            : "索引中"
          : task.state === "queued"
            ? "等待索引"
            : "等待接入／恢复",
      );
    }
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
          if (active) {
            setPreferenceUnavailable(true);
            setPreferenceError(`设置读取失败：${String(error)}`);
          }
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
    if (saveLock.current) return false;
    saveLock.current = true;
    setSaving(true);
    const ok = await writePreferences(next).then(
      (value) => {
        setPreferences(value);
        setPreferenceError("");
        return true;
      },
      (e) => {
        setPreferenceError(`设置未保存：${String(e)}`);
        return false;
      },
    );
    setSaving(false);
    saveLock.current = false;
    return ok;
  }

  return (
    <Visuals>
      <FileOperations showStatus={preferences.show_status_bar ?? true}>
        <main className="desktop">
          <Workspace
            onSettings={setSettings}
            onExit={() => {
              setExitError("");
              void invoke("app_exit").catch((error) =>
                setExitError(String(error)),
              );
            }}
            assets={assets}
            includePath={preferences.include_path}
            defaultView={preferences.default_view}
            defaultPreview={preferences.default_preview}
            searchOnlineOnly={preferences.search_online_only}
            searchIncludeHistory={preferences.search_include_history}
            showStatus={preferences.show_status_bar ?? true}
            onShowStatus={
              loaded && !saving && !preferenceUnavailable
                ? (show_status_bar) =>
                    void save({ ...preferences, show_status_bar })
                : undefined
            }
            categories={preferences.categories}
            browseFilter={preferences.browse_filter}
            filterPresets={preferences.filter_presets}
            onBrowseFilter={(id) =>
              void save({ ...preferences, browse_filter: id })
            }
            onSavePresets={(filter_presets) =>
              save({ ...preferences, filter_presets })
            }
            savingFilters={saving || !loaded}
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
              (exitError || (!settings && preferenceError)) && (
                <p role="alert" className="desktop-error preference-error">
                  {exitError || preferenceError}
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
                  indexActivityLabels={indexActivityLabels}
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
              ...(Development && {
                development: (
                  <Suspense fallback={<p>正在加载验证工具…</p>}>
                    <Development />
                  </Suspense>
                ),
              }),
            }}
          />
          {settings && (
            <Settings
              initialPage={settings}
              preferences={preferences}
              saving={saving}
              preferenceUnavailable={preferenceUnavailable || !loaded}
              preferenceError={preferenceError}
              onSave={(changes) => save({ ...preferences, ...changes })}
              onClose={() => setSettings(null)}
              tasks={tasks.data}
              taskError={tasks.error}
              onTasksRefresh={tasks.refresh}
            />
          )}
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
