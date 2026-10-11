import { useCallback, useState } from "react";
import { apiClient, type CommentRow } from "./api";
import { EmptyState, ErrorState, Skeleton, explainError, useAsync } from "./ui";

export interface RecordCommentsProps {
  appSlug: string;
  recordId: string;
  /** The signed-in person, so they can remove their own comment. */
  viewerEppn: string;
}

function when(iso: string): string {
  return iso.replace("T", " ").replace("Z", "").slice(0, 16);
}

/** The text after the last `@` the person is typing, or null when they are not typing a mention. */
function mentionQuery(text: string): string | null {
  const at = text.lastIndexOf("@");
  if (at === -1) return null;
  const rest = text.slice(at + 1);
  return rest.includes("]") || rest.includes(" ") ? null : rest;
}

export function RecordComments({ appSlug, recordId, viewerEppn }: RecordCommentsProps) {
  const [text, setText] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const load = useCallback(async () => {
    const [list, people] = await Promise.all([apiClient.listComments(appSlug, recordId), apiClient.mentionable(appSlug)]);
    return { comments: list.comments, people: people.people };
  }, [appSlug, recordId]);
  const { data, error, loading, reload } = useAsync(load);

  const query = mentionQuery(text);
  const matches =
    query === null || !data
      ? []
      : data.people.filter((p) => p.name.toLowerCase().includes(query.toLowerCase()) || p.eppn.toLowerCase().includes(query.toLowerCase()));

  const pick = (eppn: string) => {
    const at = text.lastIndexOf("@");
    setText(`${text.slice(0, at)}@[${eppn}] `);
  };

  const post = async () => {
    setProblem(null);
    try {
      await apiClient.postComment(appSlug, recordId, text);
      setText("");
      await reload();
    } catch (e) {
      setProblem(explainError(e).message);
    }
  };

  const remove = async (c: CommentRow) => {
    setProblem(null);
    try {
      await apiClient.removeComment(appSlug, c.id);
      await reload();
    } catch (e) {
      setProblem(explainError(e).message);
    }
  };

  if (loading && !data) return <Skeleton />;
  if (error) return <ErrorState error={error} onRetry={() => void reload()} />;

  return (
    <section aria-label="Comments" className="space-y-4">
      {data && data.comments.length === 0 ? (
        <EmptyState title="No comments yet" description="Write the first one below." />
      ) : (
        <ul className="space-y-3">
          {data?.comments.map((c) => (
            <li key={c.id} className="rounded-md border border-gray-200 dark:border-gray-700 p-3 text-sm">
              <div className="flex items-baseline justify-between gap-2 text-xs text-gray-500 dark:text-gray-400">
                <span className="font-medium text-gray-900 dark:text-gray-100">{c.author_name}</span>
                <time dateTime={c.created_at}>{when(c.created_at)}</time>
              </div>
              {c.deleted ? (
                <p className="mt-1 italic text-gray-500 dark:text-gray-400">Comment removed</p>
              ) : (
                <>
                  <p className="mt-1 whitespace-pre-wrap text-gray-800 dark:text-gray-200">{c.body}</p>
                  {c.author.toLowerCase() === viewerEppn.toLowerCase() && (
                    <button type="button" className="mt-1 text-xs text-red-600 hover:underline" onClick={() => void remove(c)}>
                      Remove
                    </button>
                  )}
                </>
              )}
            </li>
          ))}
        </ul>
      )}

      <div className="relative">
        <label className="block text-xs font-medium text-gray-700 dark:text-gray-300" htmlFor="new-comment">
          Write a comment
        </label>
        <textarea
          id="new-comment"
          className="mt-1 w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 p-2 text-sm"
          rows={3}
          maxLength={4000}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        {matches.length > 0 && (
          <ul role="listbox" aria-label="People you can mention" className="absolute z-10 mt-1 w-64 rounded-md border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 shadow">
            {matches.map((p) => (
              <li key={p.eppn} role="option" aria-selected={false} className="cursor-pointer px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-800" onClick={() => pick(p.eppn)}>
                {p.name}
              </li>
            ))}
          </ul>
        )}
        {problem && (
          <p role="alert" className="mt-1 text-xs text-red-600">
            {problem}
          </p>
        )}
        <button
          type="button"
          disabled={text.trim() === ""}
          className="mt-2 rounded-md bg-blue-600 px-3 py-1.5 text-sm font-semibold text-white disabled:opacity-50"
          onClick={() => void post()}
        >
          Comment
        </button>
      </div>
    </section>
  );
}
