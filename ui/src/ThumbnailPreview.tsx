import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { EntryRef } from "./file-operations";
import type { FileRow } from "./browse-api";
import { useFileVisual, useVisuals } from "./Visuals";
import { visualPhases } from "./visual-api";
import FileIcon from "./FileIcon";

export default function ThumbnailPreview({
  file,
  reference,
  connection,
}: {
  file: FileRow;
  reference: EntryRef;
  connection: string | null;
}) {
  const context = useVisuals();
  const folder = file.kind === "folder";
  const visual = useFileVisual(
    folder ? undefined : reference,
    "thumbnail",
    512,
    connection || "unconfirmed",
  );
  const [broken, setBroken] = useState<string | null>(null);
  const [ffmpeg, setFfmpeg] = useState<boolean | null>(null);
  const [repair, setRepair] = useState<{
    state: "running" | "cancelling" | "done" | "failed";
    message: string;
  }>();
  const request = useRef<{ id: string; cancelled: boolean } | null>(null);
  const image = visual.image;
  const url = image?.data_url;
  const visible = url && url !== broken;
  const outdated = [
    "索引记录已变化，请刷新重选",
    "索引代次已变化，请刷新重选",
    "文件记录已不存在，请刷新",
  ].includes(visual.error);
  const error =
    (outdated ? "" : visual.error) ||
    (url && url === broken ? "缩略图缓存无法显示" : "");
  const canRepair =
    !!context &&
    !folder &&
    connection === "online" &&
    (image?.source === "unsupported" || !!error);
  useEffect(() => {
    if (!canRepair) return;
    let live = true;
    void invoke<boolean>("visual_ffmpeg_available").then(
      (available) => {
        if (live) setFfmpeg(available);
      },
      () => {
        if (live) setFfmpeg(false);
      },
    );
    return () => {
      live = false;
    };
  }, [canRepair]);
  useEffect(
    () => () => {
      const current = request.current;
      request.current = null;
      if (current)
        void invoke("visual_cancel", { requestId: current.id }).catch(() => {});
    },
    [],
  );
  async function rebuild() {
    if (request.current) return;
    const current = { id: crypto.randomUUID(), cancelled: false };
    request.current = current;
    setRepair({ state: "running", message: "FFmpeg 正在生成缩略图…" });
    try {
      await invoke("visual_ffmpeg_rebuild", {
        requestId: current.id,
        reference,
      });
      if (request.current === current)
        setRepair({ state: "done", message: "缩略图已补建" });
    } catch (error) {
      if (request.current === current)
        setRepair(
          current.cancelled
            ? { state: "done", message: "已取消补建" }
            : {
                state: "failed",
                message: String(error).replace(/^Error: /, ""),
              },
        );
    } finally {
      if (request.current === current) request.current = null;
    }
  }
  if (!context) return null;
  const status =
    error ||
    visualPhases[visual.phase] ||
    image?.warning ||
    (image?.historical && visible
      ? "历史缓存"
      : image?.source === "offline"
        ? "离线，无缓存缩略图"
        : image?.source === "unsupported"
          ? "系统未提供缩略图"
          : "");
  return (
    <div className="thumbnail-preview">
      <div className="thumbnail-image">
        {visible ? (
          <img
            src={url}
            alt={`${file.name} 的缩略图`}
            draggable={false}
            onError={() => setBroken(url)}
          />
        ) : (
          <FileIcon
            name={file.name}
            folder={folder}
            reference={outdated ? undefined : reference}
            connection={connection || "unconfirmed"}
            size={64}
          />
        )}
      </div>
      {(repair?.message || status) && (
        <p
          role={
            (repair ? repair.state === "failed" : !!error) ? "alert" : "status"
          }
        >
          {repair?.message || status}
        </p>
      )}
      {repair?.state === "running" || repair?.state === "cancelling" ? (
        <button
          type="button"
          disabled={repair.state === "cancelling"}
          onClick={() => {
            const current = request.current;
            if (!current) return;
            current.cancelled = true;
            setRepair({ state: "cancelling", message: "正在取消补建…" });
            void invoke("visual_cancel", { requestId: current.id }).catch(
              (error) => {
                if (request.current !== current) return;
                current.cancelled = false;
                setRepair({
                  state: "running",
                  message: String(error).replace(/^Error: /, ""),
                });
              },
            );
          }}
        >
          取消补建
        </button>
      ) : canRepair && ffmpeg === true ? (
        <button type="button" onClick={() => void rebuild()}>
          用 FFmpeg 补建
        </button>
      ) : canRepair && ffmpeg === false ? (
        <p>未在 PATH 中找到 FFmpeg</p>
      ) : null}
    </div>
  );
}
