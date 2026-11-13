import {
  useId,
  useLayoutEffect,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import WorkspaceIcon from "./WorkspaceIcon";
import "./modal.css";

export default function Modal({
  title,
  children,
  actions,
  className = "",
  role,
  onClose,
  closeDisabled = false,
  initialFocus,
}: {
  title: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
  className?: string;
  role?: "alertdialog";
  onClose: () => void;
  closeDisabled?: boolean;
  initialFocus?: RefObject<HTMLElement | null>;
}) {
  const titleId = useId();
  const ref = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = ref.current!;
    const previous = document.activeElement;
    dialog.showModal();
    const focus =
      initialFocus?.current ??
      dialog.querySelector<HTMLButtonElement>(".modal-close");
    focus?.focus();
    return () => {
      dialog.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus();
    };
  }, [initialFocus]);

  return createPortal(
    <dialog
      ref={ref}
      className={`modal ${className}`}
      role={role}
      aria-labelledby={titleId}
      aria-modal="true"
      closedby={closeDisabled ? "none" : "closerequest"}
      onCancel={(event) => {
        event.preventDefault();
        if (!closeDisabled) onClose();
      }}
    >
      <header className="modal-titlebar">
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="modal-close"
          aria-label="关闭"
          title="关闭（Esc）"
          disabled={closeDisabled}
          onClick={onClose}
        >
          <WorkspaceIcon name="close" />
        </button>
      </header>
      <div className="modal-body">{children}</div>
      {actions && <div className="dialog-actions">{actions}</div>}
    </dialog>,
    document.body,
  );
}
