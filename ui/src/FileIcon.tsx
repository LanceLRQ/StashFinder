import { useState } from "react";
import type { EntryRef } from "./file-operations";
import { useFileVisual, useTypeIcon } from "./Visuals";
import WorkspaceIcon from "./WorkspaceIcon";

export default function FileIcon({
  name,
  folder,
  reference,
  connection,
  size = 32,
  active = true,
  thumbnail = false,
}: {
  name: string;
  folder: boolean;
  reference?: EntryRef;
  connection?: string;
  size?: number;
  active?: boolean;
  thumbnail?: boolean;
}) {
  const iconSize = Math.min(256, Math.ceil(size * window.devicePixelRatio));
  const type = useTypeIcon(name, folder, iconSize, active);
  const specific = folder || /\.(exe|dll|lnk|ico|cur)$/i.test(name);
  const file = useFileVisual(
    active && (thumbnail ? !folder : specific) ? reference : undefined,
    thumbnail ? "thumbnail" : "icon",
    thumbnail ? size : iconSize,
    connection,
    thumbnail,
  );
  const [broken, setBroken] = useState<string | null>(null);
  const custom = file.image?.data_url;
  const url =
    custom && custom !== broken
      ? custom
      : type && type !== broken
        ? type
        : null;
  return (
    <span
      className={
        thumbnail
          ? "thumbnail-art"
          : `file-icon ${size > 32 ? "large-file-icon" : ""}`
      }
      style={thumbnail ? { width: size, height: size } : undefined}
      aria-hidden="true"
    >
      {url ? (
        <img
          src={url}
          alt=""
          draggable={false}
          onError={() => setBroken(url)}
        />
      ) : folder ? (
        <WorkspaceIcon name="folder" className="folder-placeholder" />
      ) : (
        <span className="file-mark" />
      )}
      {thumbnail && connection === "online" && file.phase && (
        <span className="index-spinner thumbnail-pending" />
      )}
    </span>
  );
}
