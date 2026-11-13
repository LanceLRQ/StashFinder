import { useRef, useState } from "react";
import type { SearchBookmark } from "./search-options";
import Modal from "./Modal";

export default function ManageBookmarks({
  bookmarks,
  onSave,
  onClose,
}: {
  bookmarks: SearchBookmark[];
  onSave: (bookmarks: SearchBookmark[]) => Promise<string | null>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState(bookmarks);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const saving = useRef(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const changed =
    draft.length !== bookmarks.length ||
    draft.some(
      (bookmark, index) => bookmark.name.trim() !== bookmarks[index].name,
    );
  const invalid = draft.some(({ name }) => !name.trim());
  async function save() {
    if (saving.current || invalid || !changed) return;
    saving.current = true;
    setBusy(true);
    setError("");
    const failure = await onSave(
      draft.map((bookmark) => ({ ...bookmark, name: bookmark.name.trim() })),
    );
    saving.current = false;
    setBusy(false);
    if (failure) setError(failure);
    else onClose();
  }
  return (
    <Modal
      title="管理搜索书签"
      className="manage-bookmarks-dialog"
      initialFocus={cancel}
      onClose={onClose}
      closeDisabled={busy}
      actions={
        <>
          <button ref={cancel} type="button" disabled={busy} onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="primary"
            disabled={busy || invalid || !changed}
            onClick={() => void save()}
          >
            保存
          </button>
        </>
      }
    >
      {draft.length === 0 ? (
        <p>没有搜索书签</p>
      ) : (
        <ul className="bookmark-list">
          {draft.map((bookmark, index) => {
            const originalName =
              bookmarks.find(({ id }) => id === bookmark.id)?.name ??
              bookmark.name;
            return (
              <li key={bookmark.id}>
                <input
                  aria-label={`书签名称：${originalName}`}
                  value={bookmark.name}
                  maxLength={80}
                  disabled={busy}
                  onChange={(e) => {
                    const name = e.target.value;
                    setDraft((items) =>
                      items.map((item) =>
                        item.id === bookmark.id ? { ...item, name } : item,
                      ),
                    );
                  }}
                />
                <button
                  type="button"
                  aria-label={`移除 ${originalName}`}
                  disabled={busy}
                  onClick={(e) => {
                    const inputs = e.currentTarget
                      .closest("ul")
                      ?.querySelectorAll("input");
                    (
                      inputs?.[index + 1] ??
                      inputs?.[index - 1] ??
                      cancel.current
                    )?.focus();
                    setDraft((items) =>
                      items.filter(({ id }) => id !== bookmark.id),
                    );
                  }}
                >
                  移除
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {error && (
        <p role="alert" className="desktop-error">
          {error}
        </p>
      )}
    </Modal>
  );
}
