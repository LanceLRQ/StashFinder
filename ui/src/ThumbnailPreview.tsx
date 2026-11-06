import { useState } from "react";
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
  if (!context) return null;
  const image = visual.image;
  const url = image?.data_url;
  const visible = url && url !== broken;
  const error =
    visual.error || (url && url === broken ? "缩略图缓存无法显示" : "");
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
            reference={reference}
            connection={connection || "unconfirmed"}
            size={64}
          />
        )}
      </div>
      {status && <p role={error ? "alert" : "status"}>{status}</p>}
    </div>
  );
}
