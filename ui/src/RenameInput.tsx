import { useEffect, useRef, useState } from "react";
export default function RenameInput({
  name,
  folder,
  onSubmit,
  onCancel,
}: {
  name: string;
  folder: boolean;
  onSubmit: (value: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(name);
  useEffect(() => {
    ref.current?.focus();
    const dot = name.lastIndexOf(".");
    ref.current?.setSelectionRange(0, !folder && dot > 0 ? dot : name.length);
  }, [name, folder]);
  return (
    <input
      className="file-rename-input"
      aria-label="新名称"
      ref={ref}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={onCancel}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.nativeEvent.isComposing) return;
        if (e.key === "Enter") {
          e.preventDefault();
          if (value === name) onCancel();
          else onSubmit(value);
        }
        if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
    />
  );
}
