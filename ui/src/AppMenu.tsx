import { useEffect, useRef, useState } from "react";

export type MenuGroup = {
  label: string;
  items: {
    label: string;
    run: () => void;
    disabled?: boolean;
    shortcut?: string;
  }[];
};

export default function AppMenu({ groups }: { groups: MenuGroup[] }) {
  const [open, setOpen] = useState<number | null>(null);
  const root = useRef<HTMLElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (open === null) return;
    function outside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) setOpen(null);
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  return (
    <nav
      ref={root}
      className="app-menu"
      aria-label="程序菜单"
      onKeyDown={(event) => {
        if (open === null || event.key !== "Escape") return;
        event.preventDefault();
        setOpen(null);
        trigger.current?.focus();
      }}
    >
      {groups.map((group, index) => (
        <div className="app-menu-group" key={group.label}>
          <button
            aria-haspopup="menu"
            aria-expanded={open === index}
            onClick={(event) => {
              trigger.current = event.currentTarget;
              setOpen(open === index ? null : index);
            }}
          >
            {group.label}
          </button>
          {open === index && (
            <div
              className="app-menu-popup"
              role="menu"
              aria-label={group.label}
            >
              {group.items.map((item) => (
                <button
                  key={item.label}
                  role="menuitem"
                  aria-disabled={item.disabled}
                  onClick={() => {
                    if (item.disabled) return;
                    setOpen(null);
                    trigger.current?.focus();
                    item.run();
                  }}
                >
                  {item.label}
                  {item.shortcut && (
                    <span aria-hidden="true">{item.shortcut}</span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
    </nav>
  );
}
