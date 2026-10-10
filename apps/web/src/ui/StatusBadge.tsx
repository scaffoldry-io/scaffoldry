export type BadgeTone = "ok" | "warn" | "bad" | "neutral";

export interface StatusBadgeProps {
  tone: BadgeTone;
  text: string;
}

export function StatusBadge({ tone, text }: StatusBadgeProps) {
  const config = {
    ok: {
      style: "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-950/30 dark:text-emerald-300 dark:border-emerald-800",
      icon: "●",
    },
    warn: {
      style: "bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-950/30 dark:text-amber-300 dark:border-amber-800",
      icon: "▲",
    },
    bad: {
      style: "bg-red-50 text-red-700 border-red-200 dark:bg-red-950/30 dark:text-red-300 dark:border-red-800",
      icon: "■",
    },
    neutral: {
      style: "bg-gray-100 text-gray-700 border-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:border-gray-700",
      icon: "○",
    },
  }[tone];

  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium border ${config.style}`}
    >
      <span aria-hidden="true" className="text-[10px] select-none">
        {config.icon}
      </span>
      <span>{text}</span>
    </span>
  );
}
