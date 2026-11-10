import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuGroup = {
  label: string;
  accessKey: string;
  items: {
    label: string;
    run: () => void;
    disabled?: boolean;
    shortcut?: string;
    checked?: boolean;
  }[];
};

export default function AppMenu({ groups }: { groups: MenuGroup[] }) {
  const [open, setOpen] = useState<number | null>(null);
  const root = useRef<HTMLElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const previous = useRef<HTMLElement | null>(null);
  const focusLast = useRef(false);
  const pointerOrigin = useRef<HTMLElement | null>(null);
  const enabledItems = () =>
    Array.from(
      popup.current?.querySelectorAll<HTMLButtonElement>(
        'button:not([aria-disabled="true"])',
      ) ?? [],
    );
  function close(restore: boolean) {
    setOpen(null);
    if (restore && previous.current?.isConnected) previous.current.focus();
  }
  function show(index: number, from: HTMLElement | null, last = false) {
    if (open === null) previous.current = from;
    focusLast.current = last;
    if (open === index)
      (enabledItems().at(last ? -1 : 0) ?? popup.current)?.focus();
    else setOpen(index);
  }
  useLayoutEffect(() => {
    if (open === null) return;
    (enabledItems().at(focusLast.current ? -1 : 0) ?? popup.current)?.focus();
  }, [open]);
  useEffect(() => {
    const outside = (event: PointerEvent) => {
      if (open !== null && !root.current?.contains(event.target as Node))
        close(false);
    };
    const key = (event: KeyboardEvent) => {
      if (
        !event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        event.isComposing ||
        event.keyCode === 229 ||
        event.defaultPrevented ||
        document.querySelector(
          'dialog[open], [role="dialog"], [role="alertdialog"]',
        )
      )
        return;
      const group = groups.findIndex(
        (item) => item.accessKey === event.key.toLowerCase(),
      );
      if (group < 0) return;
      event.preventDefault();
      show(group, document.activeElement as HTMLElement);
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", key);
    };
  }, [groups, open]);
  return (
    <nav ref={root} className="app-menu" aria-label="程序菜单">
      {groups.map((group, index) => (
        <div className="app-menu-group" key={group.label}>
          <button
            aria-haspopup="menu"
            aria-expanded={open === index}
            aria-keyshortcuts={`Alt+${group.accessKey.toUpperCase()}`}
            onPointerDown={() => {
              pointerOrigin.current = document.activeElement as HTMLElement;
            }}
            onClick={(event) => {
              if (open === index) close(true);
              else show(index, pointerOrigin.current ?? event.currentTarget);
              pointerOrigin.current = null;
            }}
            onKeyDown={(event) => {
              if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
              event.preventDefault();
              show(index, event.currentTarget, event.key === "ArrowUp");
            }}
          >
            {group.label}
          </button>
          {open === index && (
            <div
              ref={popup}
              className="app-menu-popup"
              role="menu"
              aria-label={group.label}
              tabIndex={-1}
              onKeyDown={(event) => {
                const { key } = event;
                if (key === "Escape" || key === "Tab") {
                  if (key === "Escape") event.preventDefault();
                  close(true);
                } else if (key === "ArrowLeft" || key === "ArrowRight") {
                  event.preventDefault();
                  show(
                    (index + (key === "ArrowRight" ? 1 : groups.length - 1)) %
                      groups.length,
                    null,
                  );
                } else if (
                  ["ArrowDown", "ArrowUp", "Home", "End"].includes(key)
                ) {
                  event.preventDefault();
                  const items = enabledItems();
                  const current = items.indexOf(
                    document.activeElement as HTMLButtonElement,
                  );
                  const next =
                    key === "Home"
                      ? 0
                      : key === "End"
                        ? items.length - 1
                        : current < 0
                          ? key === "ArrowDown"
                            ? 0
                            : items.length - 1
                          : (current +
                              (key === "ArrowDown" ? 1 : items.length - 1)) %
                            items.length;
                  items[next]?.focus();
                } else if (key === "Enter" || key === " ") {
                  event.preventDefault();
                  if (event.target instanceof HTMLButtonElement)
                    event.target.click();
                }
              }}
            >
              {group.items.map((item) => (
                <button
                  key={item.label}
                  role={
                    item.checked === undefined ? "menuitem" : "menuitemcheckbox"
                  }
                  tabIndex={-1}
                  aria-checked={item.checked}
                  aria-disabled={item.disabled}
                  onClick={() => {
                    if (item.disabled) return;
                    close(true);
                    item.run();
                  }}
                >
                  {item.checked === undefined ? (
                    item.label
                  ) : (
                    <span className="menu-check">
                      <span aria-hidden="true">{item.checked ? "✓" : ""}</span>
                      {item.label}
                    </span>
                  )}
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
