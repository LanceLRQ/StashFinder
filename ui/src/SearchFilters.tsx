import { useState } from "react";
import Modal from "./Modal";
import {
  dateBoundary,
  dateLabel,
  emptyOptions,
  hasFileFilter,
  noFilter,
  selectFilter,
  type Category,
  type FileFilter,
  type FilterPreset,
  type FilterSelection,
  type SearchOptions,
} from "./search-options";
export default function SearchFilters({
  initialOptions,
  initialFilter,
  categories,
  initialNamesOnly,
  defaultNamesOnly,
  presets,
  onSavePresets,
  savingPresets,
  onApply,
  onClose,
}: {
  initialOptions: SearchOptions;
  initialFilter: FilterSelection;
  categories: Category[];
  initialNamesOnly: boolean;
  defaultNamesOnly: boolean;
  presets: FilterPreset[];
  onSavePresets?: (presets: FilterPreset[]) => Promise<boolean>;
  savingPresets: boolean;
  onApply: (
    filter: FilterSelection,
    options: SearchOptions,
    namesOnly: boolean,
  ) => void;
  onClose: () => void;
}) {
  const [options, setOptions] = useState(initialOptions);
  const [filter, setFilter] = useState(initialFilter);
  const [namesOnly, setNamesOnly] = useState(initialNamesOnly);
  const selectedPreset = presets.find((p) => p.id === filter.id);
  const [name, setName] = useState(selectedPreset?.name ?? "");
  const [error, setError] = useState("");
  const { rule, types, custom } = filter;
  const update = (key: keyof SearchOptions, value: unknown) =>
    setOptions({ ...options, [key]: value });
  const updateRule = (next: Partial<FileFilter>) =>
    setFilter({ ...filter, rule: { ...rule, ...next } });
  function changeTypes(nextTypes: string[], nextCustom: string) {
    const extensions = [
      ...new Set([
        ...categories
          .filter((c) => nextTypes.includes(`category:${c.name}`))
          .flatMap((c) => c.extensions),
        ...nextCustom
          .split(/[,;]/)
          .map((part) => part.trim())
          .filter(Boolean),
      ]),
    ];
    const folder = nextTypes.includes("@folder");
    setFilter({
      ...filter,
      types: nextTypes,
      custom: nextCustom,
      rule: {
        ...rule,
        extensions,
        include_folders: folder && extensions.length > 0,
        kind: folder
          ? extensions.length
            ? null
            : "folder"
          : rule.kind === "file"
            ? "file"
            : null,
      },
    });
  }
  async function savePreset(replace: boolean) {
    if (!onSavePresets) return;
    const preset = {
      id: (replace && selectedPreset?.id) || `user:${crypto.randomUUID()}`,
      name: name.trim(),
      rule,
    };
    const ok = await onSavePresets(
      replace
        ? presets.map((p) => (p.id === preset.id ? preset : p))
        : [...presets, preset],
    );
    setError(ok ? "" : "预设未保存，请检查名称、扩展名和范围。");
    if (ok)
      setFilter((current) =>
        current === filter ? { ...current, id: preset.id } : current,
      );
  }
  return (
    <Modal
      title="筛选器"
      className="search-filter-dialog"
      onClose={onClose}
      actions={
        <>
          <button
            type="button"
            onClick={() => {
              setFilter(noFilter);
              setName("");
              setError("");
              setNamesOnly(defaultNamesOnly);
              setOptions({
                ...emptyOptions,
                sort: options.sort,
                descending: options.descending,
              });
            }}
          >
            清除筛选
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => onApply(filter, options, namesOnly)}
          >
            应用
          </button>
        </>
      }
    >
      <div className="search-filter-content">
        <fieldset>
          <legend>系统预设</legend>
          <div className="search-filter-types">
            {[
              { id: "@folder", name: "文件夹" },
              ...categories.map((c) => ({
                id: `category:${c.name}`,
                name: c.name,
              })),
            ].map(({ id, name }) => (
              <label key={id}>
                <input
                  type="checkbox"
                  checked={types.includes(id)}
                  onChange={(e) =>
                    changeTypes(
                      e.target.checked
                        ? [...types, id]
                        : types.filter((t) => t !== id),
                      custom,
                    )
                  }
                />
                {name}
              </label>
            ))}
          </div>
        </fieldset>
        {onSavePresets && (
          <fieldset className="search-filter-fields">
            <legend>我的预设</legend>
            <label>
              选用预设
              <select
                aria-label="我的预设"
                value={selectedPreset?.id ?? ""}
                onChange={(e) => {
                  const preset = presets.find((p) => p.id === e.target.value);
                  setFilter(
                    preset
                      ? selectFilter(preset.id, categories, presets)
                      : noFilter,
                  );
                  setName(preset?.name ?? "");
                  setError("");
                }}
              >
                <option value="">未选择</option>
                {presets.map((preset) => (
                  <option value={preset.id} key={preset.id}>
                    {preset.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              预设名称
              <input
                aria-label="预设名称"
                maxLength={80}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <div className="search-filter-preset-actions">
              <button
                type="button"
                disabled={savingPresets || !name.trim() || !hasFileFilter(rule)}
                onClick={() => void savePreset(false)}
              >
                另存为预设
              </button>
              {selectedPreset && (
                <>
                  <button
                    type="button"
                    disabled={
                      savingPresets || !name.trim() || !hasFileFilter(rule)
                    }
                    onClick={() => void savePreset(true)}
                  >
                    保存修改
                  </button>
                  <button
                    type="button"
                    disabled={savingPresets}
                    onClick={() => {
                      void onSavePresets(
                        presets.filter((p) => p.id !== selectedPreset.id),
                      ).then((ok) => {
                        setError(ok ? "" : "未能移除预设。");
                        if (ok) {
                          setFilter({ ...filter, id: undefined });
                          setName("");
                        }
                      });
                    }}
                  >
                    移除预设
                  </button>
                </>
              )}
            </div>
            {error && (
              <p className="search-filter-preset-actions" role="alert">
                {error}
              </p>
            )}
          </fieldset>
        )}
        <fieldset className="search-filter-fields">
          <legend>文件条件</legend>
          <label>
            扩展名
            <input
              aria-label="自定义扩展名"
              placeholder="mkv,mp4"
              value={custom}
              onChange={(e) => changeTypes(types, e.target.value)}
            />
          </label>
          <label>
            <input
              type="checkbox"
              checked={rule.kind === "file"}
              onChange={(e) =>
                setFilter({
                  ...filter,
                  types: types.filter((id) => id !== "@folder"),
                  rule: {
                    ...rule,
                    include_folders: false,
                    kind: e.target.checked ? "file" : null,
                  },
                })
              }
            />
            仅文件
          </label>
          {(["min", "max"] as const).map((key) => (
            <label key={key}>
              {key === "min" ? "最小字节数" : "最大字节数"}
              <input
                type="number"
                min="0"
                value={rule.size[key] || ""}
                onChange={(e) =>
                  updateRule({
                    size: { ...rule.size, [key]: e.target.value || null },
                  })
                }
              />
            </label>
          ))}
          {(["modified", "created"] as const).map((field) => (
            <fieldset className="search-filter-date" key={field}>
              <legend>{field === "modified" ? "修改日期" : "创建日期"}</legend>
              {(["min", "max"] as const).map((key) => (
                <input
                  key={key}
                  aria-label={`${field === "modified" ? "修改" : "创建"}${key === "min" ? "起始日期" : "结束日期"}`}
                  type="date"
                  value={dateLabel(rule[field][key])}
                  min="1601-01-01"
                  max="9999-12-30"
                  onChange={(e) =>
                    updateRule({
                      [field]: {
                        ...rule[field],
                        [key]: dateBoundary(e.target.value, key === "max"),
                      },
                    })
                  }
                />
              ))}
            </fieldset>
          ))}
        </fieldset>
        <fieldset className="search-filter-fields">
          <legend>本次搜索</legend>
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
                ["extension", "扩展名"],
                ["type", "类型"],
                ["modified", "修改时间"],
                ["created", "创建时间"],
                ["attributes", "属性"],
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
        </fieldset>
      </div>
    </Modal>
  );
}
