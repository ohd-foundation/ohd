import type { ReactNode } from "react";

/**
 * Bottom-sheet on mobile, centered dialog on desktop. Click-outside or Escape
 * closes. Mirrors the modal shell from `care/web` but with mobile-first
 * affordances (sheet rounding, sticky footer).
 */
export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  subtitle?: string;
  children: ReactNode;
  footer?: ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="modal-overlay"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
    >
      <div className="modal" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <div className="modal-head">
          <h3 id="modal-title">{title}</h3>
          {subtitle ? <div className="modal-sub">{subtitle}</div> : null}
        </div>
        <div className="modal-body">{children}</div>
        {footer ? <div className="modal-foot">{footer}</div> : null}
      </div>
    </div>
  );
}
