import type { ReactNode } from "react";

export interface EmptyStateProps {
  title: string;
  description: string;
  action?: ReactNode;
}

export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center p-8 text-center bg-gray-50 dark:bg-gray-900/50 rounded-lg border border-dashed border-gray-300 dark:border-gray-700">
      <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">
        {title}
      </h3>
      <p className="mt-1 text-sm text-gray-500 dark:text-gray-400 max-w-sm">
        {description}
      </p>
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
