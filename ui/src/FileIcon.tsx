import { useState } from "react";
import type { EntryRef } from "./file-operations";
import { useFileVisual, useTypeIcon } from "./Visuals";

export default function FileIcon({
  name,
  folder,
  reference,
  connection,
  size = 32,
  active = true,
}: {
  name: string;
  folder: boolean;
  reference?: EntryRef;
  connection?: string;
  size?: number;
  active?: boolean;
}) {
  const type = useTypeIcon(name, folder, size, active);
  const specific = folder || /\.(exe|dll|lnk|ico|cur)$/i.test(name);
  const file = useFileVisual(
    specific && active ? reference : undefined,
    "icon",
    size,
    connection,
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
      className={`file-icon ${size > 32 ? "large-file-icon" : ""}`}
      aria-hidden="true"
    >
      {url ? (
        <img
          src={url}
          alt=""
          draggable={false}
          onError={() => setBroken(url)}
        />
      ) : (
        <span className={`file-mark ${folder ? "folder" : "file"}`} />
      )}
    </span>
  );
}
