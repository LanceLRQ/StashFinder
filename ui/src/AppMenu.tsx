import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuCommand = {
  id?: string;
  label: string;
  run: () => void;
  disabled?: boolean;
  shortcut?: string;
  checked?: boolean;
  radio?: boolean;
};

type MenuItem =
  | MenuCommand
  | {
      label: string;
      items: MenuCommand[];
      disabled?: boolean;
    };

export type MenuGroup = {
  label: string;
  accessKey: string;
  items: MenuItem[];
};

export default function AppMenu({ groups }: { groups: MenuGroup[] }) {
  const [open, setOpen] = useState<number | null>(null);
  const [child, setChild] = useState<number | null>(null);
  const [tabStop, setTabStop] = useState(0);
  const root = useRef<HTMLElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const submenu = useRef<HTMLDivElement>(null);
  const triggers = useRef<(HTMLButtonElement | null)[]>([]);
  const previous = useRef<HTMLElement | null>(null);
  const focusLast = useRef(false);
  const pointerOrigin = useRef<HTMLElement | null>(null);
  const enabledItems = (menu: HTMLDivElement | null) =>
    Array.from(
      menu?.querySelectorAll<HTMLButtonElement>(
        ':scope > button:not([aria-disabled="true"]), :scope > div > button:not([aria-disabled="true"])',
      ) ?? [],
    );

  function close(restore: boolean) {
    setOpen(null);
    setChild(null);
    if (restore) {
      const source = previous.current;
      (source?.isConnected && !source.closest("[hidden], [inert]")
        ? source
        : triggers.current[tabStop]
      )?.focus();
    }
  }

  function show(index: number, from: HTMLElement | null, last = false) {
    if (open === null) previous.current = from;
    setTabStop(index);
    setChild(null);
    focusLast.current = last;
    if (open === index)
      (enabledItems(popup.current).at(last ? -1 : 0) ?? popup.current)?.focus();
    else setOpen(index);
  }

  function closeChild() {
    const trigger = submenu.current
      ?.previousElementSibling as HTMLElement | null;
    setChild(null);
    trigger?.focus();
  }

  useLayoutEffect(() => {
    if (open !== null)
      (
        enabledItems(popup.current).at(focusLast.current ? -1 : 0) ??
        popup.current
      )?.focus();
  }, [open]);
  useLayoutEffect(() => {
    const menu = submenu.current;
    const trigger = menu?.previousElementSibling;
    if (child === null || !menu || !trigger) return;
    const position = () => {
      const anchor = trigger.getBoundingClientRect();
      const { offsetWidth, offsetHeight } = menu;
      const left =
        anchor.right + offsetWidth <= window.innerWidth
          ? anchor.right
          : anchor.left - offsetWidth;
      menu.style.left = `${Math.max(0, left)}px`;
      menu.style.top = `${Math.max(0, Math.min(anchor.top, window.innerHeight - offsetHeight))}px`;
    };
    position();
    (enabledItems(menu)[0] ?? menu).focus();
    const parent = popup.current;
    parent?.addEventListener("scroll", position);
    window.addEventListener("resize", position);
    return () => {
      parent?.removeEventListener("scroll", position);
      window.removeEventListener("resize", position);
    };
  }, [open, child]);

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
        event.key === "Process" ||
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
      if (!event.repeat) show(group, document.activeElement as HTMLElement);
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", key);
    };
  }, [groups, open]);

  function handleMenuKey(
    event: React.KeyboardEvent<HTMLDivElement>,
    nested: boolean,
    groupIndex: number,
  ) {
    const { key } = event;
    if (
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.nativeEvent.isComposing
    )
      return;
    event.stopPropagation();
    const target = event.target as HTMLButtonElement;
    if (key === "Tab") {
      close(false);
      triggers.current[groupIndex]?.focus();
      return;
    }
    if (key === "Escape" || (nested && key === "ArrowLeft")) {
      event.preventDefault();
      if (nested) closeChild();
      else close(true);
    } else if (key === "ArrowRight" || key === "ArrowLeft") {
      event.preventDefault();
      if (
        !nested &&
        key === "ArrowRight" &&
        target.dataset.submenu !== undefined
      ) {
        if (target.getAttribute("aria-disabled") !== "true")
          setChild(Number(target.dataset.submenu));
      } else if (!nested)
        show(
          (groupIndex + (key === "ArrowRight" ? 1 : groups.length - 1)) %
            groups.length,
          null,
        );
    } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(key)) {
      event.preventDefault();
      const items = enabledItems(event.currentTarget);
      const current = items.indexOf(target);
      const next =
        key === "Home"
          ? 0
          : key === "End"
            ? items.length - 1
            : current < 0
              ? key === "ArrowDown"
                ? 0
                : items.length - 1
              : (current + (key === "ArrowDown" ? 1 : items.length - 1)) %
                items.length;
      items[next]?.focus();
    } else if (key === "Enter" || key === " ") {
      event.preventDefault();
      if (!event.repeat && target instanceof HTMLButtonElement) target.click();
    }
  }

  function command(item: MenuCommand) {
    return (
      <button
        key={item.id ?? item.label}
        type="button"
        tabIndex={-1}
        role={
          item.radio
            ? "menuitemradio"
            : item.checked === undefined
              ? "menuitem"
              : "menuitemcheckbox"
        }
        aria-checked={item.checked}
        aria-disabled={item.disabled}
        aria-keyshortcuts={item.shortcut
          ?.replace("↑", "ArrowUp")
          .replace("↓", "ArrowDown")
          .replace("←", "ArrowLeft")
          .replace("→", "ArrowRight")}
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
        {item.shortcut && <span aria-hidden="true">{item.shortcut}</span>}
      </button>
    );
  }

  return (
    <nav ref={root} className="app-menu" aria-label="程序菜单">
      {groups.map((group, index) => (
        <div className="app-menu-group" key={group.label}>
          <button
            ref={(element) => {
              triggers.current[index] = element;
            }}
            type="button"
            tabIndex={tabStop === index ? 0 : -1}
            aria-haspopup="menu"
            aria-expanded={open === index}
            aria-keyshortcuts={`Alt+${group.accessKey.toUpperCase()}`}
            onFocus={() => setTabStop(index)}
            onPointerDown={() => {
              pointerOrigin.current = document.activeElement as HTMLElement;
            }}
            onClick={(event) => {
              if (open === index) close(true);
              else show(index, pointerOrigin.current ?? event.currentTarget);
              pointerOrigin.current = null;
            }}
            onKeyDown={(event) => {
              const { key } = event;
              if (key === "ArrowDown" || key === "ArrowUp") {
                event.preventDefault();
                show(index, event.currentTarget, key === "ArrowUp");
              } else if (key === "ArrowLeft" || key === "ArrowRight") {
                event.preventDefault();
                const next =
                  (index + (key === "ArrowRight" ? 1 : groups.length - 1)) %
                  groups.length;
                triggers.current[next]?.focus();
              }
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
              onKeyDown={(event) => handleMenuKey(event, false, index)}
            >
              {group.items.map((item, position) =>
                "items" in item ? (
                  <div className="app-menu-branch" key={item.label} role="none">
                    <button
                      type="button"
                      role="menuitem"
                      tabIndex={-1}
                      data-submenu={position}
                      aria-haspopup="menu"
                      aria-expanded={child === position}
                      aria-disabled={item.disabled}
                      onClick={() => {
                        if (!item.disabled) setChild(position);
                      }}
                    >
                      {item.label}
                      <span className="app-menu-arrow" aria-hidden="true" />
                    </button>
                    {child === position && !item.disabled && (
                      <div
                        ref={submenu}
                        className="app-menu-popup app-menu-submenu"
                        role="menu"
                        aria-label={item.label}
                        tabIndex={-1}
                        onKeyDown={(event) => handleMenuKey(event, true, index)}
                      >
                        {item.items.map(command)}
                      </div>
                    )}
                  </div>
                ) : (
                  command(item)
                ),
              )}
            </div>
          )}
        </div>
      ))}
    </nav>
  );
}
