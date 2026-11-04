// Process v2 phase 5: what a step, a committee's vote and a due date say in words. Pure functions, so the builder, the
// desk and their tests read the same sentences.
import type { ApproverSpec, WorkflowPredicate } from "./types";

/** How many must agree on a step: one person, everyone, or at least some number. */
export type DecisionChoice = "Single" | "AllOf" | { AtLeast: number };

/** What happens when a step is late. A timer never approves or rejects. */
export type LateChoice = "" | "Remind" | "EscalateUp" | "Fail";

/** Who a step is asked of, in the words the server's `describe_rule` uses. */
export function approverWords(spec: ApproverSpec): string {
  if ("Position" in spec) {
    const unit = spec.Position.from === "Submitter" ? "the submitter's unit" : "the workspace's unit";
    return `the ${spec.Position.key} of ${unit}`;
  }
  if ("Person" in spec) return spec.Person.eppn;
  return `the people with the ${spec.Role.name} role`;
}

const LATE_WORDS: Record<Exclude<LateChoice, "">, string> = {
  Remind: "they are reminded",
  EscalateUp: "it goes to the next level up",
  Fail: "it fails",
};

/** The preview sentence for a whole approval step. */
export function stepPreview(approver: ApproverSpec, decision: DecisionChoice, dueDays: number, late: LateChoice): string {
  const parts = [`Asked of ${approverWords(approver)}.`];
  if (decision === "AllOf") parts.push("All of them must approve.");
  if (typeof decision === "object") parts.push(`At least ${decision.AtLeast} of them must approve.`);
  if (dueDays > 0 && late) parts.push(`If no one decides in ${dueDays} ${dueDays === 1 ? "day" : "days"}, ${LATE_WORDS[late]}.`);
  return parts.join(" ");
}

/** A committee's vote so far, as counts. It names nobody. */
export function tallyWords(t: { approve: number; reject: number; voters: number }): string {
  return `${t.approve} of ${t.voters} ${t.approve === 1 && t.voters === 1 ? "has" : "have"} approved`;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** When a step is due: the date, and how far away or how late that is. */
export function dueWords(fireAt: string, now: Date = new Date()): string {
  const at = new Date(fireAt);
  const date = at.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
  const days = Math.round((at.getTime() - now.getTime()) / 86_400_000);
  if (days > 0) return `Due ${date}, in ${plural(days, "day")}`;
  if (days === 0) return `Due ${date}, today`;
  return `Due ${date}, ${plural(-days, "day")} overdue`;
}

const OPERATOR_WORDS: Record<string, string> = {
  Equals: "is",
  NotEquals: "is not",
  GreaterThan: "is greater than",
  LessThan: "is less than",
  Contains: "contains",
};

/** A condition in words: `gpa is greater than 3.5`. */
export function predicateWords(p: WorkflowPredicate): string {
  return `${p.field_name} ${OPERATOR_WORDS[p.operator] ?? p.operator} ${p.expected_value}`;
}

/** A step's action in words. */
export function actionWords(action: Record<string, any>): string {
  if (action.UpdateRecordStatus) return `Set the status to ${action.UpdateRecordStatus.new_status}.`;
  if (action.NotifyCollaborator) return `Tell the ${action.NotifyCollaborator.role}: ${action.NotifyCollaborator.message_template}.`;
  if (action.CreateLedgerAuditEntry) return `Record in the ledger: ${action.CreateLedgerAuditEntry.summary}.`;
  if (action.WebhookDispatch) return `Send a webhook to ${action.WebhookDispatch.target_url}.`;
  return "Do something the builder cannot show yet.";
}
