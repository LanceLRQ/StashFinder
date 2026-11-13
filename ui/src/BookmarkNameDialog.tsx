import { useRef, useState } from "react";
import Modal from "./Modal";

export default function BookmarkNameDialog({
  title,
  initialName,
  onSave,
  onClose,
}: {
  title: string;
  initialName: string;
  onSave: (name: string) => Promise<string | null>;
  onClose: () => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const saving = useRef(false);
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState(initialName);
  const [error, setError] = useState("");
  async function save() {
    if (saving.current || !name.trim()) return;
    saving.current = true;
    setBusy(true);
    setError("");
    const failure = await onSave(name.trim());
    saving.current = false;
    setBusy(false);
    if (failure) setError(failure);
    else onClose();
  }
  return (
    <Modal
      title={title}
      initialFocus={input}
      onClose={onClose}
      closeDisabled={busy}
      actions={
        <>
          <button type="button" disabled={busy} onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="primary"
            disabled={busy || !name.trim()}
            onClick={() => void save()}
          >
            保存
          </button>
        </>
      }
    >
      <label className="setting-row">
        书签名称
        <input
          ref={input}
          value={name}
          maxLength={80}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.nativeEvent.isComposing &&
              e.keyCode !== 229 &&
              !e.repeat
            ) {
              e.preventDefault();
              void save();
            }
          }}
        />
      </label>
      {error && (
        <p role="alert" className="desktop-error">
          {error}
        </p>
      )}
    </Modal>
  );
}
