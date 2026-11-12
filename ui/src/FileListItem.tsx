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
  editing,
  location,
  badge,
  children,
  ...events
}: HTMLAttributes<HTMLElement> & {
  view: FileView;
  index: number;
  total: number;
  selected: boolean;
  file?: Pick<FileRow, "name" | "kind">;
  active: boolean;
  reference?: EntryRef;
  connection?: string;
  editing?: {
    onSubmit: (name: string) => void;
    onCancel: () => void;
  };
  location?: string;
  badge?: string;
}) {
  const grid = view === "details" ? undefined : thumbnailLayouts[view];
  const Item = grid ? "div" : "tr";
  const icon = file && (
    <FileIcon
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
                {name}
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
              {name}
              {badge && <small className="file-entry-badge">{badge}</small>}
            </td>
          )}
          {children}
        </>
      )}
    </Item>
  );
}
