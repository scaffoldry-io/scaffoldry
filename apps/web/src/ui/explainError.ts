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
    case "too_large":
      return { message: e.message, next: "Narrow the filter." };
    default:
      return e.remedy ? { message: e.message, next: e.remedy } : { message: e.message };
  }
}
