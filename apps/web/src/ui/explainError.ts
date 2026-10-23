import { ApiError } from "../api";

export interface ExplainedError {
  message: string;
  next?: string;
}

const ASK_FOR_ACCESS = "Ask a workspace owner or an administrator if you need access.";

/** A server that sends no `code` is read by its status. */
function codeFromStatus(status: number): string | undefined {
  switch (status) {
    case 401:
      return "unauthorized";
    case 403:
      return "forbidden";
    case 404:
      return "not_found";
    case 409:
      return "version_conflict";
    case 413:
      return "too_large";
    default:
      return undefined;
  }
}

/**
 * The one place an error becomes words. Every `ErrorState` and failed `Toast` uses it.
 * A code a later brief adds gets a row here in the same change. Until it has one, the
 * server's sentence and its `remedy` are shown.
 */
export function explainError(e: unknown): ExplainedError {
  if (!(e instanceof ApiError)) {
    return { message: e instanceof Error ? e.message : String(e) };
  }
  if (e.status === 0) {
    return { message: "The server is not reachable.", next: "Check the connection and try again." };
  }
  switch (e.code ?? codeFromStatus(e.status)) {
    case "unauthorized":
      return { message: "Your session has ended.", next: "Sign in again." };
    case "forbidden":
      return e.policy
        ? { message: e.policy.description, next: ASK_FOR_ACCESS }
        : { message: "You do not have permission to do this.", next: ASK_FOR_ACCESS };
    case "version_conflict":
      return { message: "Someone else changed this first.", next: "Your view has been refreshed. Review it and try again." };
    case "no_approver":
      return { message: "No one is assigned to decide this step.", next: "Ask an administrator to assign a position holder." };
    case "position_full":
      return { message: "This position already has its holders.", next: "Replace a current holder, or raise the limit on the position type." };
    case "undo_conflict":
      return { message: "Someone changed this since. Undo stopped." };
    case "view_as_read_only":
      return { message: "You are viewing as someone else. This view is read-only.", next: "Exit the view to make changes." };
    case "quota_exceeded":
      return { message: e.message, next: "Wait for your background work to finish, or cancel some of it, then try again." };
    case "too_many_indexes": {
      const unused: string[] = Array.isArray(e.details?.unused) ? e.details.unused : [];
      const next = unused.length
        ? `Ask an admin to stop indexing a field no view uses, such as ${unused.slice(0, 3).join(", ")}.`
        : "Sort or filter on a field that is already indexed.";
      return { message: e.message, next };
    }
    case "calculating":
      return { message: e.message, next: "Try again when it has finished." };
    case "index_building":
      return { message: e.message, next: "Try again in a few minutes." };
    case "field_hidden":
    case "field_read_only":
      // The rule's own sentence, and who to ask. A rule gives a reason; the server makes one when it does not.
      return { message: e.reason ?? e.message, next: e.remedy ?? ASK_FOR_ACCESS };
    case "constraint_violated":
      return { message: e.message, next: "Check the field value and try again." };
    case "too_large":
      return { message: e.message, next: "Narrow the filter." };
    case "file_too_large":
      return { message: e.message, next: "Choose a smaller file, or ask an admin to raise the file size limit." };
    case "grant_outside_scope":
      return { message: "You can only give access inside your own units, and never above admin.", next: "Choose a unit you administer, or ask a Platform Admin." };
    case "grant_too_broad":
      return { message: "That would give access to everyone below the top of a hierarchy.", next: "Anchor it on one workspace as a viewer, or use the workspace's institutional visibility." };
    case "proposal_changed":
      return { message: "This proposal is not what you reviewed.", next: "Open it again, read what it now holds, and approve that." };
    case "storage_unavailable":
      return { message: "The server cannot reach its database right now.", next: "Try again in a minute. If it keeps happening, tell an administrator." };
    case "scanner_unavailable":
      return { message: "Files cannot be uploaded right now.", next: "Try again in a few minutes, or ask an admin to check the virus scanner." };
    case "file_type_blocked":
      return { message: e.message, next: "Save it as a PDF, an image, or a plain document, then upload that." };
    default:
      return e.remedy ? { message: e.message, next: e.remedy } : { message: e.message };
  }
}
