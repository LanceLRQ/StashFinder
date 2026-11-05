import { useEffect, useRef, useState } from "react";
import { readDirectory, type DirectoryPage, type Ancestor } from "./browse-api";

export default function DirectoryTree({
  partition,
  filesystem,
  name,
  entry = null,
  selected,
  ancestors,
  version,
  onNavigate,
}: {
  partition: string;
  filesystem: string;
  name: string;
  entry?: number[] | null;
  selected: number[] | null;
  onNavigate: (id: number[] | null) => void;
  ancestors?: Ancestor[];
  version?: string;
}) {
  const [expanded, setExpanded] = useState(entry === null);
  const [page, setPage] = useState<DirectoryPage | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const request = useRef(0);
  useEffect(() => {
    if (entry && ancestors?.some((a) => a.entry.join(",") === entry.join(",")))
      setExpanded(true);
  }, [ancestors, entry]);
  useEffect(() => {
    if (!expanded) return;
    const token = ++request.current;
    setLoading(true);
    setError("");
    void readDirectory({
      partition_id: partition,
      filesystem_id: filesystem,
      parent: entry,
      folders_only: true,
    }).then(
      (value) => {
        if (token === request.current) {
          setPage(value);
          setLoading(false);
        }
      },
      (e) => {
        if (token === request.current) {
          setError(String(e));
          setLoading(false);
        }
      },
    );
    return () => {
      request.current++;
    };
  }, [expanded, partition, filesystem, entry, refresh, version]);
  async function more() {
    if (!page || page.next === null || loading) return;
    const token = request.current;
    setLoading(true);
    await readDirectory({
      partition_id: partition,
      filesystem_id: filesystem,
      parent: page.location,
      folders_only: true,
      generation: page.generation,
      revision: page.revision,
      offset: page.next,
    }).then(
      (value) => {
        if (token === request.current)
          setPage({ ...value, entries: [...page.entries, ...value.entries] });
      },
      (e) => {
        if (token === request.current) setError(String(e));
      },
    );
    if (token === request.current) setLoading(false);
  }
  return (
    <div className="tree-node">
      <div className="tree-row">
        <button
          className="tree-expander"
          aria-label={`${expanded ? "收起" : "展开"}${name}`}
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? "⌄" : "›"}
        </button>
        <button
          className="tree-label"
          title={name}
          aria-current={
            JSON.stringify(entry || page?.location) === JSON.stringify(selected)
              ? "location"
              : undefined
          }
          onClick={() => onNavigate(entry)}
        >
          <span className="file-mark folder" aria-hidden="true" />
          {name}
        </button>
      </div>
      {expanded && (
        <div className="tree-children">
          {error && (
            <div className="tree-error">
              <span>{error}</span>
              <button onClick={() => setRefresh((n) => n + 1)}>重试</button>
            </div>
          )}
          {page?.entries.map((row) => (
            <DirectoryTree
              key={row.entry.join(",")}
              partition={partition}
              filesystem={filesystem}
              name={row.name}
              entry={row.entry}
              selected={selected}
              ancestors={ancestors}
              version={version}
              onNavigate={onNavigate}
            />
          ))}
          {page?.next != null && (
            <button disabled={loading} onClick={() => void more()}>
              更多目录
            </button>
          )}
          {loading && <span className="stage-note">正在读取…</span>}
        </div>
      )}
    </div>
  );
}
