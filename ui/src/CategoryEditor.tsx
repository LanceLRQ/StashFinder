import { useEffect, useState } from "react";
import { defaultCategories, type Category } from "./search-options";
export default function CategoryEditor({
  categories = defaultCategories,
  saving,
  onSave,
}: {
  categories?: Category[];
  saving: boolean;
  onSave: (categories: Category[]) => Promise<void>;
}) {
  const [editing, setEditing] = useState<number | null>(null);
  const [name, setName] = useState("");
  const [extensions, setExtensions] = useState("");
  const [error, setError] = useState("");
  useEffect(() => setEditing(null), [categories]);
  async function save() {
    const normalized = [
      ...new Set(
        extensions
          .split(/[,;]/)
          .map((e) => e.trim().replace(/^\.+/, "").toLowerCase())
          .filter(Boolean),
      ),
    ];
    if (
      !name.trim() ||
      name.trim().length > 64 ||
      categories.some(
        (c, i) =>
          i !== editing && c.name.toLowerCase() === name.trim().toLowerCase(),
      ) ||
      !normalized.length ||
      normalized.some((e) => !/^[\p{L}\p{N}_+-]+$/u.test(e))
    ) {
      setError("分类名称不能重复，扩展名使用逗号分隔且不含路径或通配符。");
      return;
    }
    const next = [...categories];
    const value = { name: name.trim(), extensions: normalized };
    if (editing === categories.length) next.push(value);
    else next[editing!] = value;
    await onSave(next);
  }
  return (
    <section className="category-editor">
      <div className="activity-heading">
        <h2>文件类型</h2>
        <button
          disabled={saving || editing !== null}
          onClick={() => {
            setEditing(categories.length);
            setName("");
            setExtensions("");
            setError("");
          }}
        >
          新增类型
        </button>
      </div>
      {categories.map((c, i) => (
        <div className="category-row" key={c.name}>
          <strong>{c.name}</strong>
          <span title={c.extensions.join(", ")}>{c.extensions.join(", ")}</span>
          <button
            disabled={saving}
            onClick={() => {
              setEditing(i);
              setName(c.name);
              setExtensions(c.extensions.join(", "));
              setError("");
            }}
          >
            编辑 {c.name}
          </button>
          <button
            disabled={saving}
            onClick={() => void onSave(categories.filter((_, n) => n !== i))}
          >
            移除 {c.name}
          </button>
        </div>
      ))}
      {editing !== null && (
        <form
          className="category-form"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label>
            分类名称
            <input
              aria-label="分类名称"
              value={name}
              maxLength={64}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            扩展名
            <input
              aria-label="分类扩展名"
              value={extensions}
              placeholder="jpg,png"
              onChange={(e) => setExtensions(e.target.value)}
            />
          </label>
          {error && <p role="alert">{error}</p>}
          <button disabled={saving}>保存类型</button>
          <button type="button" onClick={() => setEditing(null)}>
            取消
          </button>
        </form>
      )}
    </section>
  );
}
