import { useEffect, useState } from "react";

/** A job as the server describes it to the person who started it. */
export interface BackgroundJob {
  id: string;
  kind: string;
  /** A plain name, such as `Recalculating a field`. */
  title: string;
  /** One plain sentence. */
  description: string;
  state: string;
  progress_done: number;
  progress_total: number | null;
}

export interface BackgroundWorkProps {
  /** Returns the signed-in person's recent jobs. */
  fetchJobs: () => Promise<BackgroundJob[]>;
  /** Until `realtime.md` phase 3 pushes changes, the indicator asks every five seconds. */
  intervalMs?: number;
}

const ACTIVE = ["queued", "running"];

function progressWords(job: BackgroundJob): string {
  if (job.state === "queued" || !job.progress_total) return "Waiting";
  return `${Math.min(99, Math.round((job.progress_done / job.progress_total) * 100))}%`;
}

/** The signed-in person's queued and running jobs. It shows nothing when there are none. */
export function BackgroundWork({ fetchJobs, intervalMs = 5000 }: BackgroundWorkProps) {
  const [jobs, setJobs] = useState<BackgroundJob[]>([]);

  useEffect(() => {
    let alive = true;
    const poll = () => {
      fetchJobs()
        .then((list) => {
          if (alive) setJobs(list.filter((j) => ACTIVE.includes(j.state)));
        })
        // A failed poll keeps what is shown. The next one tries again.
        .catch(() => {});
    };
    poll();
    const timer = setInterval(poll, intervalMs);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [fetchJobs, intervalMs]);

  if (jobs.length === 0) return null;
  return (
    <div role="status" className="rounded-lg border border-gray-200 dark:border-gray-700 p-3 text-sm">
      <p className="font-semibold">Background work</p>
      <ul className="mt-1 space-y-1">
        {jobs.map((job) => (
          <li key={job.id}>
            <span>{job.title}</span> <span className="text-gray-600 dark:text-gray-300">{progressWords(job)}</span>
            <p className="text-xs text-gray-600 dark:text-gray-300">{job.description}</p>
          </li>
        ))}
      </ul>
    </div>
  );
}
