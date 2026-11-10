import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
export function virtualScale(total: number, height: number) {
  return Math.max(
    1,
    (total * 64 - height) / Math.max(1, Math.min(total * 64, 8000000) - height),
  );
}
export function virtualWindow(
  total: number,
  logicalTop: number,
  height: number,
) {
  const scale = virtualScale(total, height);
  const clamped = Math.min(logicalTop, Math.max(0, total * 64 - height));
  const start = Math.max(0, Math.floor(clamped / 64) - 8);
  const end = Math.min(total, start + Math.ceil(height / 64) + 16);
  return { start, end, top: clamped / scale, logicalTop: clamped };
}
export default function VirtualResults({
  total,
  revision,
  onRange,
  onPosition,
  restore,
  row,
  onKeyDown,
  onSort,
  sort,
  descending,
}: {
  total: number;
  revision: number;
  onRange: (start: number, end: number) => void;
  onPosition: (logicalTop: number, height: number, scrolling: boolean) => void;
  restore: { logicalTop: number } | null;
  row: (index: number) => ReactNode;
  onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => void;
  onSort?: (field: string) => void;
  sort?: string;
  descending?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(500);
  const content = Math.min(total * 64, 8000000);
  const scale = virtualScale(total, height);
  const physicalTop = Math.min(top, Math.max(0, content - height));
  const { start, end } = virtualWindow(total, physicalTop * scale, height);
  useEffect(() => {
    const element = ref.current;
    if (!element || scale === 1) return;
    const wheel = (e: WheelEvent) => {
      if (e.deltaY) {
        e.preventDefault();
        element.scrollTop += e.deltaY / scale;
      }
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [scale]);
  useLayoutEffect(() => {
    setTop(0);
    if (ref.current) ref.current.scrollTop = 0;
  }, [revision]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!restore || !element) return;
    const viewportHeight = element.clientHeight || height;
    element.scrollTop = virtualWindow(
      total,
      restore.logicalTop,
      viewportHeight,
    ).top;
    setTop(element.scrollTop);
    onPosition(
      element.scrollTop * virtualScale(total, viewportHeight),
      viewportHeight,
      false,
    );
  }, [restore, total]);
  useEffect(() => {
    onRange(start, end);
  }, [start, end, onRange]);
  useEffect(() => {
    const element = ref.current!;
    const measure = () =>
      setHeight((previous) => element.clientHeight || previous);
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const element = ref.current;
    if (element) onPosition(element.scrollTop * scale, height, false);
  }, [height, revision]);
  return (
    <div
      className="virtual-results"
      aria-label="搜索结果列表"
      ref={ref}
      tabIndex={0}
      onScroll={(e) => {
        const { scrollTop, clientHeight } = e.currentTarget;
        setTop(scrollTop);
        const viewportHeight = clientHeight || height;
        onPosition(
          scrollTop * virtualScale(total, viewportHeight),
          viewportHeight,
          true,
        );
      }}
      onKeyDown={onKeyDown}
    >
      <table aria-rowcount={total + 1}>
        <thead>
          <tr>
            {[
              "名称",
              "所在目录 / 路径",
              "分区",
              "大小",
              "修改时间",
              "状态 / 索引时间",
              "重复组",
            ].map((s, i) => (
              <th
                key={s}
                aria-sort={
                  sort ===
                  [
                    "name",
                    "path",
                    "partition",
                    "size",
                    "modified",
                    "",
                    "group",
                  ][i]
                    ? descending
                      ? "descending"
                      : "ascending"
                    : undefined
                }
              >
                {i === 5 ? (
                  s
                ) : (
                  <button
                    aria-label={`按${s}排序`}
                    onClick={() =>
                      onSort?.(
                        [
                          "name",
                          "path",
                          "partition",
                          "size",
                          "modified",
                          "",
                          "group",
                        ][i],
                      )
                    }
                  >
                    {s}
                  </button>
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          <tr
            aria-hidden="true"
            style={{ height: Math.max(0, physicalTop - 8 * 64) }}
          >
            <td colSpan={7} />
          </tr>
          {Array.from({ length: end - start }, (_, i) => row(start + i))}
          <tr
            aria-hidden="true"
            style={{
              height: Math.max(
                0,
                content -
                  Math.max(0, physicalTop - 8 * 64) -
                  (end - start) * 64,
              ),
            }}
          >
            <td colSpan={7} />
          </tr>
        </tbody>
      </table>
    </div>
  );
}
