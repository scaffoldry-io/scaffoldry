import React from "react";
import type { ProcessStepItem } from "./types";
import { actionWords, approverWords, predicateWords } from "./processWords";

// Process v2 phase 5: a rule's steps read back in words, with a branch's steps nested under each of its conditions.
export const RuleSteps: React.FC<{ steps: ProcessStepItem[]; nested?: boolean }> = ({ steps, nested }) => (
  <ol data-testid={nested ? "rule-steps-nested" : "rule-steps"} className={`space-y-1 text-xs text-slate-700 dark:text-slate-300 ${nested ? "pl-4 border-l border-slate-200 dark:border-slate-700" : ""}`}>
    {steps.map((step) => (
      <li key={step.id}>
        {step.when.length > 0 && <span className="text-slate-500">If {step.when.map(predicateWords).join(" and ")}: </span>}
        <StepWords step={step} />
      </li>
    ))}
  </ol>
);

const StepWords: React.FC<{ step: ProcessStepItem }> = ({ step }) => {
  const kind = step.kind;
  if ("Service" in kind) return <span>{actionWords(kind.Service.action)}</span>;
  if ("UserTask" in kind) {
    const task = kind.UserTask;
    const who = task.approver ? approverWords(task.approver) : `the ${task.role}`;
    return <span>Wait for a decision by {who}.</span>;
  }
  return (
    <div className="space-y-1">
      {kind.Branch.branches.map((arm, i) => (
        <div key={i} data-testid={`branch-arm-${i}`}>
          <div className="font-medium">{i === 0 ? "If" : "Otherwise, if"} {arm.when.map(predicateWords).join(" and ") || "always"}:</div>
          <RuleSteps steps={arm.steps} nested />
        </div>
      ))}
      {kind.Branch.otherwise.length > 0 && (
        <div data-testid="branch-otherwise">
          <div className="font-medium">Otherwise:</div>
          <RuleSteps steps={kind.Branch.otherwise} nested />
        </div>
      )}
    </div>
  );
};
