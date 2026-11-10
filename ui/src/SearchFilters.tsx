import {
  dateBoundary,
  dateLabel,
  emptyOptions,
  type Category,
  type SearchOptions,
} from "./search-options";
export default function SearchFilters({
  options,
  setOptions,
  categories,
  types,
  setTypes,
  custom,
  setCustom,
  namesOnly,
  setNamesOnly,
}: {
  options: SearchOptions;
  setOptions: (o: SearchOptions) => void;
  categories: Category[];
  types: string[];
  setTypes: (s: string[]) => void;
  custom: string;
  setCustom: (s: string) => void;
  namesOnly: boolean;
  setNamesOnly: (value: boolean) => void;
}) {
  const update = (key: keyof SearchOptions, value: unknown) =>
    setOptions({ ...options, [key]: value });
  const count =
    Number(options.case_sensitive) +
    Number(options.mode !== "contains") +
    Number(options.duplicate.length > 0) +
    Number(options.online_only) +
    Number(options.include_history) +
    Number(!!options.size.min || !!options.size.max) +
    Number(!!options.modified.min || !!options.modified.max) +
    Number(!!options.created.min || !!options.created.max);
  return (
    <>
      <details className="view-menu type-filter">
        <summary>{types.length ? `类型 ${types.length}` : "全部类型"}</summary>
        <div>
          <label>
            <input
              type="checkbox"
              checked={types.includes("@folder")}
              onChange={(e) =>
                setTypes(
                  e.target.checked
                    ? [...types, "@folder"]
                    : types.filter((t) => t !== "@folder"),
                )
              }
            />
            文件夹
          </label>
          {categories.map((c) => (
            <label key={c.name}>
              <input
                type="checkbox"
                checked={types.includes(`category:${c.name}`)}
                onChange={(e) =>
                  setTypes(
                    e.target.checked
                      ? [...types, `category:${c.name}`]
                      : types.filter((t) => t !== `category:${c.name}`),
                  )
                }
              />
              {c.name}
            </label>
          ))}
          <label>
            扩展名
            <input
              aria-label="自定义扩展名"
              placeholder="mkv,mp4"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
            />
          </label>
        </div>
      </details>
      <details className="view-menu search-filters">
        <summary>筛选{count ? ` ${count}` : ""}</summary>
        <div>
          <label>
            <input
              type="checkbox"
              checked={namesOnly}
              onChange={(e) => setNamesOnly(e.target.checked)}
            />
            仅文件名
          </label>
          <label>
            匹配
            <select
              aria-label="匹配方式"
              value={options.mode}
              onChange={(e) => update("mode", e.target.value)}
            >
              {[
                ["contains", "包含"],
                ["whole_name", "完整文件名"],
                ["whole_word", "完整单词"],
                ["start", "文件名开头"],
                ["end", "文件名末尾"],
                ["wildcard", "通配符"],
                ["regex", "正则表达式"],
              ].map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </select>
          </label>
          <label>
            <input
              type="checkbox"
              checked={options.case_sensitive}
              onChange={(e) => update("case_sensitive", e.target.checked)}
            />
            区分大小写
          </label>
          <label>
            重复项
            <select
              aria-label="重复项"
              value={options.duplicate.join(";")}
              onChange={(e) =>
                setOptions({
                  ...options,
                  duplicate: e.target.value ? e.target.value.split(";") : [],
                  sort: e.target.value ? "group" : "name",
                })
              }
            >
              {[
                ["", "不限"],
                ["name", "同名"],
                ["size", "同大小"],
                ["name;size", "同名且同大小"],
                ["modified", "同修改时间"],
                ["name;size;modified", "同名、大小及修改时间"],
              ].map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </select>
          </label>
          {(["min", "max"] as const).map((key) => (
            <label key={key}>
              {key === "min" ? "最小字节数" : "最大字节数"}
              <input
                aria-label={key === "min" ? "最小字节数" : "最大字节数"}
                type="number"
                min="0"
                value={options.size[key] || ""}
                onChange={(e) =>
                  update("size", {
                    ...options.size,
                    [key]: e.target.value || null,
                  })
                }
              />
            </label>
          ))}
          {(["modified", "created"] as const).map((field) => (
            <fieldset key={field}>
              <legend>{field === "modified" ? "修改日期" : "创建日期"}</legend>
              {(["min", "max"] as const).map((key) => (
                <input
                  key={key}
                  aria-label={`${field === "modified" ? "修改" : "创建"}${key === "min" ? "起始日期" : "结束日期"}`}
                  type="date"
                  value={dateLabel(options[field][key])}
                  min="1601-01-01"
                  max="9999-12-30"
                  onChange={(e) =>
                    update(field, {
                      ...options[field],
                      [key]: dateBoundary(e.target.value, key === "max"),
                    })
                  }
                />
              ))}
            </fieldset>
          ))}
          <label>
            <input
              type="checkbox"
              checked={options.online_only}
              onChange={(e) => update("online_only", e.target.checked)}
            />
            仅在线
          </label>
          <label>
            <input
              type="checkbox"
              checked={options.include_history}
              onChange={(e) => update("include_history", e.target.checked)}
            />
            包含历史记录
          </label>
          <label>
            排序
            <select
              aria-label="搜索排序"
              value={options.sort}
              onChange={(e) => update("sort", e.target.value)}
            >
              {[
                ["name", "名称"],
                ["path", "路径"],
                ["partition", "分区"],
                ["size", "大小"],
                ["modified", "修改时间"],
                ["created", "创建时间"],
                ["group", "重复组"],
              ].map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </select>
          </label>
          <label>
            <input
              type="checkbox"
              checked={options.descending}
              onChange={(e) => update("descending", e.target.checked)}
            />
            降序
          </label>
          <button
            onClick={() => {
              setOptions({
                ...emptyOptions,
                sort: options.sort,
                descending: options.descending,
              });
              setTypes([]);
              setCustom("");
            }}
          >
            清除筛选
          </button>
        </div>
      </details>
    </>
  );
}
