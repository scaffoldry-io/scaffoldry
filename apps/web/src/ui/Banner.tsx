import type { ReactNode } from "react";

export type BannerKind = "info" | "warning" | "error";

export interface BannerProps {
  kind: BannerKind;
  children: ReactNode;
  icon?: ReactNode;
}

export function Banner({ kind, children, icon }: BannerProps) {
  const role = kind === "error" ? "alert" : "status";

  const config = {
    info: {
      bg: "bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-900 text-blue-900 dark:text-blue-200",
      defaultIcon: "ℹ",
    },
    warning: {
      bg: "bg-amber-50 dark:bg-amber-950/30 border-amber-200 dark:border-amber-900 text-amber-900 dark:text-amber-200",
      defaultIcon: "⚠",
    },
    error: {
      bg: "bg-red-50 dark:bg-red-950/30 border-red-200 dark:border-red-900 text-red-900 dark:text-red-200",
      defaultIcon: "✕",
    },
  }[kind];

  return (
    <div
      role={role}
      className={`p-3.5 rounded-lg border flex items-start gap-3 text-sm ${config.bg}`}
    >
      <span className="font-bold select-none" aria-hidden="true">
        {icon || config.defaultIcon}
      </span>
      <div className="flex-1">{children}</div>
    </div>
  );
}
