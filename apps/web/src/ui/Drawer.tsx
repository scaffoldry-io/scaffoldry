import { useEffect, useRef, type ReactNode } from "react";

export interface DrawerProps {
  open: boolean;
  onClose: () => void;
  title: string;
  actions?: ReactNode;
  children: ReactNode;
  /** `modal` centres the panel. The focus handling is the same. */
  variant?: "drawer" | "modal";
  /** Width of the panel. */
  size?: "md" | "lg" | "xl";
  testId?: string;
  closeTestId?: string;
}

const WIDTH = { md: "max-w-md", lg: "max-w-lg", xl: "max-w-4xl" } as const;

export function Drawer({
  open,
  onClose,
  title,
  actions,
  children,
  variant = "drawer",
  size = "lg",
  testId,
  closeTestId,
}: DrawerProps) {
  const drawerRef = useRef<HTMLDivElement>(null);
  const previouslyFocusedElement = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (open) {
      previouslyFocusedElement.current = document.activeElement as HTMLElement;
      const focusables = drawerRef.current?.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
      );
      if (focusables && focusables.length > 0) {
        focusables[0].focus();
      } else {
        drawerRef.current?.focus();
      }
      // Give focus back to the opener when the panel closes or goes away.
      return () => previouslyFocusedElement.current?.focus();
    }
  }, [open]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!open) return;

      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }

      if (e.key === "Tab") {
        if (!drawerRef.current) return;
        const focusables = Array.from(
          drawerRef.current.querySelectorAll<HTMLElement>(
            'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
          )
        );
        if (focusables.length === 0) return;

        const first = focusables[0];
        const last = focusables[focusables.length - 1];

        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const titleId = "drawer-title-" + title.toLowerCase().replace(/[^a-z0-9]+/g, "-");

  return (
    <div
      data-testid={testId}
      className={`fixed inset-0 z-40 flex bg-black/40 backdrop-blur-xs ${
        variant === "modal" ? "items-center justify-center p-4" : "justify-end"
      }`}
    >
      <div
        ref={drawerRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={`w-full ${WIDTH[variant === "modal" ? size : "lg"]} bg-white dark:bg-gray-900 border-gray-200 dark:border-gray-800 shadow-xl flex flex-col focus:outline-none ${
          variant === "modal" ? "max-h-[90vh] rounded-xl border" : "h-full border-l"
        }`}
      >
        <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-800">
          <h2
            id={titleId}
            className="text-lg font-semibold text-gray-900 dark:text-gray-100"
          >
            {title}
          </h2>
          <div className="flex items-center gap-2">
            {actions}
            <button
              type="button"
              onClick={onClose}
              data-testid={closeTestId}
              aria-label="Close panel"
              className="p-1 rounded text-gray-400 hover:text-gray-600 dark:hover:text-gray-200"
            >
              ✕
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto p-4">{children}</div>
      </div>
    </div>
  );
}
