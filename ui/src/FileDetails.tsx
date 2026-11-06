import { connectionLabel, fileTime, type FileRow } from "./browse-api";

export default function FileDetails({
  rows,
  partition,
  connection,
  indexedAt,
  selectionCount = rows.length,
}: {
  rows: FileRow[];
  partition: string;
  connection: string | null;
  indexedAt: number | null;
  selectionCount?: number;
}) {
  const file = selectionCount === 1 && rows.length === 1 ? rows[0] : null;
  return (
    <aside className="file-preview" aria-label="预览窗格">
      {!selectionCount ? (
        <p className="stage-note">选择文件以查看属性</p>
      ) : (
        <>
          <strong>
            {file?.name || `已选择 ${selectionCount.toLocaleString()} 项`}
          </strong>
          <dl>
            {partition && (
              <>
                <dt>分区</dt>
                <dd>{partition}</dd>
              </>
            )}
            {(selectionCount === 1 || connection) && (
              <>
                <dt>状态</dt>
                <dd>
                  {connectionLabel[connection || "unconfirmed"] || "状态未确认"}
                </dd>
              </>
            )}
            {file && (
              <>
                <dt>路径</dt>
                <dd>{file.path || "\\"}</dd>
                <dt>类型</dt>
                <dd>{file.kind === "folder" ? "文件夹" : "文件"}</dd>
                <dt>大小</dt>
                <dd>
                  {file.kind === "folder"
                    ? "—"
                    : file.size == null
                      ? "未知"
                      : `${BigInt(file.size).toLocaleString()} B`}
                </dd>
                <dt>创建时间</dt>
                <dd>{fileTime(file.created)}</dd>
                <dt>修改时间</dt>
                <dd>{fileTime(file.modified)}</dd>
              </>
            )}
            {(selectionCount === 1 || indexedAt != null) && (
              <>
                <dt>索引时间</dt>
                <dd>
                  {indexedAt == null
                    ? "未知"
                    : new Date(indexedAt * 1000).toLocaleString()}
                </dd>
              </>
            )}
          </dl>
        </>
      )}
    </aside>
  );
}
