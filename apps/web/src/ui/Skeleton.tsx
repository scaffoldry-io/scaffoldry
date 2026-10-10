export interface SkeletonProps {
  className?: string;
  rows?: number;
}

export function Skeleton({ className = "h-4 w-full", rows }: SkeletonProps) {
  if (rows && rows > 1) {
    return (
      <div className="space-y-2.5" aria-hidden="true">
        {Array.from({ length: rows }).map((_, i) => (
          <div
            key={i}
            className={`rounded bg-gray-200 dark:bg-gray-800 motion-safe:animate-pulse ${className}`}
          />
        ))}
      </div>
    );
  }

  return (
    <div
      aria-hidden="true"
      className={`rounded bg-gray-200 dark:bg-gray-800 motion-safe:animate-pulse ${className}`}
    />
  );
}
