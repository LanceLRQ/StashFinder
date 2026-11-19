import type { HTMLAttributes } from "react";
import type { FileRow } from "./browse-api";
import type { EntryRef } from "./file-operations";
import { thumbnailLayouts, type FileView } from "./file-view";
import FileIcon from "./FileIcon";
import RenameInput from "./RenameInput";

export default function FileListItem({
  view,
  index,
  total,
  selected,
  file,
  active,
  reference,
  connection,
  connectionStatus,
  editing,
  location,
  children,
  ...events
}: HTMLAttributes<HTMLElement> & {
  view: FileView;
  index: number;
  total: number;
  selected: boolean;
  file?: Pick<FileRow, "name" | "kind" | "object">;
  active: boolean;
  reference?: EntryRef;
  connection?: string;
  connectionStatus?: string;
  editing?: {
    onSubmit: (name: string) => void;
    onCancel: () => void;
  };
  location?: string;
}) {
  const grid = view === "details" ? undefined : thumbnailLayouts[view];
  const Item = grid ? "div" : "tr";
  const icon = file && (
    <FileIcon
      key={JSON.stringify(file.object)}
      active={active}
      name={file.name}
      folder={file.kind === "folder"}
      reference={reference}
      connection={connection}
      size={grid?.size}
      thumbnail={!!grid}
    />
  );
  const name =
    file &&
    (editing ? (
      <RenameInput
        name={file.name}
        folder={file.kind === "folder"}
        onSubmit={editing.onSubmit}
        onCancel={editing.onCancel}
      />
    ) : (
      file.name
    ));
  const status = connectionStatus && (
    <span className="file-connection">{connectionStatus} </span>
  );
  return (
    <Item
      {...events}
      data-index={index}
      role={grid ? "option" : undefined}
      aria-rowindex={grid ? undefined : index + 2}
      aria-posinset={grid ? index + 1 : undefined}
      aria-setsize={grid ? total : undefined}
      aria-selected={selected}
      className={grid ? "thumbnail-cell" : undefined}
      title={
        grid && file
          ? location
            ? `${file.name}\n${location}`
            : file.name
          : undefined
      }
    >
      {grid ? (
        file ? (
          <>
            {icon}
            {editing ? (
              name
            ) : (
              <span className="thumbnail-label">
                {status}
                <span>{name}</span>
                {location && (
                  <span className="thumbnail-location"> · {location}</span>
                )}
              </span>
            )}
          </>
        ) : (
          "正在读取…"
        )
      ) : (
        <>
          {file && (
            <td title={file.name}>
              {icon}
              {status}
              {status && !editing ? <span>{name}</span> : name}
            </td>
          )}
          {children}
        </>
      )}
    </Item>
  );
}
