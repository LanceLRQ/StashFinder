import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { EntryRef } from "./file-operations";
import type { VisualImage } from "./visual-api";
import ThumbnailJobs, { type ThumbnailScope } from "./ThumbnailJobs";
import "./visuals.css";

const Context = createContext<{
  epoch: number;
  icon: (name: string, folder: boolean, size: number) => Promise<VisualImage>;
  invalidate: () => void;
  openThumbnails: (scope: ThumbnailScope) => void;
  thumbnailTaskRevision: number;
} | null>(null);
export default function Visuals({ children }: { children: ReactNode }) {
  const [epoch, setEpoch] = useState(0);
  const [thumbnailView, setThumbnailView] = useState<ThumbnailScope | null>(
    null,
  );
  const [thumbnailTaskRevision, setThumbnailTaskRevision] = useState(0);
  const icons = useRef(new Map<string, Promise<VisualImage>>());
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void listen("thumbnail-cache-updated", () => {
      if (live) setEpoch((value) => value + 1);
    })
      .then((unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      })
      .catch(() => {});
    return () => {
      live = false;
      stop?.();
    };
  }, []);
  const value = useMemo(
    () => ({
      epoch,
      icon(name: string, folder: boolean, size: number) {
        const dot = name.lastIndexOf(".");
        const extension =
          folder || dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
        const key = `${folder}:${extension}:${size}`;
        const found = icons.current.get(key);
        if (found) {
          icons.current.delete(key);
          icons.current.set(key, found);
          return found;
        }
        const request = Promise.resolve()
          .then(() =>
            invoke<VisualImage>("visual_icon", { name, folder, size }),
          )
          .catch((error) => {
            if (icons.current.get(key) === request) icons.current.delete(key);
            throw error;
          });
        icons.current.set(key, request);
        if (icons.current.size > 256)
          icons.current.delete(icons.current.keys().next().value!);
        return request;
      },
      invalidate() {
        icons.current.clear();
        setEpoch((e) => e + 1);
      },
      openThumbnails: setThumbnailView,
      thumbnailTaskRevision,
    }),
    [epoch, thumbnailTaskRevision],
  );
  return (
    <Context.Provider value={value}>
      {children}
      {thumbnailView && (
        <ThumbnailJobs
          scope={thumbnailView}
          onStarted={() => {
            setThumbnailView(null);
            setThumbnailTaskRevision((revision) => revision + 1);
          }}
          onClose={() => setThumbnailView(null)}
        />
      )}
    </Context.Provider>
  );
}
export const useVisuals = () => useContext(Context);
export function useTypeIcon(
  name: string,
  folder: boolean,
  size: number,
  active = true,
) {
  const context = useVisuals();
  const key = JSON.stringify([name, folder, size, context?.epoch, active]);
  const [state, setState] = useState<{ key: string; url: string | null }>({
    key: "",
    url: null,
  });
  useEffect(() => {
    if (!context || !active) return;
    let live = true;
    void context
      .icon(name, folder, size)
      .then((image) => {
        if (live) setState({ key, url: image.data_url });
      })
      .catch(() => {
        if (live) setState({ key, url: null });
      });
    return () => {
      live = false;
    };
  }, [key, context]);
  return state.key === key ? state.url : null;
}

const thumbnailQueue = new Set<() => void>();
let thumbnailRequests = 0;
function drainThumbnails() {
  while (thumbnailRequests < 4 && thumbnailQueue.size) {
    const start = thumbnailQueue.values().next().value!;
    thumbnailQueue.delete(start);
    thumbnailRequests++;
    start();
  }
}

export function useFileVisual(
  reference: EntryRef | undefined,
  kind: "icon" | "thumbnail",
  size: number,
  connection?: string,
  queued = false,
) {
  const context = useVisuals();
  const key = JSON.stringify([
    reference,
    kind,
    size,
    connection,
    queued,
    context?.epoch,
  ]);
  const [state, setState] = useState<{
    key: string;
    image: VisualImage | null;
    phase: string;
    error: string;
  }>({ key: "", image: null, phase: "", error: "" });
  useEffect(() => {
    if (!context || !reference) return;
    let live = true;
    let done = false;
    let started = false;
    let unlisten: (() => void) | undefined;
    function stopListening() {
      unlisten?.();
      unlisten = undefined;
    }
    const requestId = crypto.randomUUID();
    setState({ key, image: null, phase: "waiting", error: "" });
    const request = () => {
      // Progress reporting is optional, even when the image request is queued.
      void listen<{ request_id: string; phase: string }>(
        "visual-progress",
        (event) => {
          if (live && !done && event.payload.request_id === requestId)
            setState((previous) => ({
              ...previous,
              phase: event.payload.phase,
            }));
        },
      ).then(
        (stop) => {
          if (live && !done) unlisten = stop;
          else stop();
        },
        () => {},
      );
      started = true;
      void invoke<VisualImage>("visual_preview", {
        requestId,
        reference,
        kind,
        size,
      })
        .then((image) => {
          if (live) {
            done = true;
            stopListening();
            setState({ key, image, phase: "", error: "" });
          }
        })
        .catch((error) => {
          if (live) {
            done = true;
            stopListening();
            setState({
              key,
              image: null,
              phase: "",
              error: String(error).replace(/^Error: /, ""),
            });
          }
        })
        .finally(() => {
          if (queued) {
            thumbnailRequests--;
            drainThumbnails();
          }
        });
    };
    if (queued) {
      thumbnailQueue.add(request);
      drainThumbnails();
    } else request();
    return () => {
      live = false;
      thumbnailQueue.delete(request);
      stopListening();
      if (started && !done)
        void invoke("visual_cancel", { requestId }).catch(() => {});
    };
  }, [key, context]);
  return state.key === key
    ? state
    : {
        key,
        image: null,
        phase: reference && context ? "waiting" : "",
        error: "",
      };
}
