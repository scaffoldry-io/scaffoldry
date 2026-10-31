import { useCallback, useEffect, useState } from "react";
import { apiClient, type ProposalCheck, type ProposalRow, type ProposalTrial } from "./api";
import { ConfirmAction, DataTable, Drawer, PageHeader, StatusBadge, type DataTableColumn } from "./ui";

/** The checks that need a second person when they fail. */
const FLAGGED = new Set(["sensitivity", "data_loss", "page_sensitivity", "page_dependencies", "rule_effects", "access_loosened", "coordinates_widened", "mask_weakened"]);

const CHECK_WORDS: Record<string, string> = {
  schema: "Manifest format",
  sensitivity: "Sensitive data",
  data_loss: "Data loss",
  stale: "Out of date",
  page_source: "Page source",
  page_sensitivity: "Page reads sensitive data",
  page_dependencies: "Bundle libraries",
  rule_schema: "Rule format",
  rule_effects: "Rule sets sensitive data",
  rule_loop: "Rule may loop",
  rule_approver: "Who decides",
  access_rules: "Access rules",
  access_loosened: "Access loosened",
  coordinates_widened: "Place widened",
  mask_weakened: "Mask shows more",
};

function tone(c: ProposalCheck): "ok" | "warn" | "bad" {
  return c.outcome === "pass" ? "ok" : c.outcome === "warn" ? "warn" : "bad";
}

function outcomeWord(c: ProposalCheck): string {
  return c.outcome === "pass" ? "passed" : c.outcome === "warn" ? "warning" : "failed";
}

function isFlagged(p: ProposalRow): boolean {
  return p.checks.some((c) => c.outcome === "fail" && FLAGGED.has(c.name));
}

function statusTone(status: ProposalRow["status"]): "ok" | "warn" | "bad" | "neutral" {
  return status === "Pending" ? "warn" : status === "Approved" ? "ok" : status === "Rejected" ? "bad" : "neutral";
}

export interface ProposalsProps {
  workspaceId: string;
  /** The signed-in person, so a proposer cannot approve a flagged change of their own. */
  viewerEppn: string;
  /** Opens the proposed app read-only. The desk swaps its token and shows a banner. */
  onPreview?: (proposal: ProposalRow) => void | Promise<void>;
  /** Opens a trial copy of the proposed app. The desk shows it as any other app. */
  onTrial?: (trial: ProposalTrial, proposal: ProposalRow) => void | Promise<void>;
}

export function Proposals({ workspaceId, viewerEppn, onPreview, onTrial }: ProposalsProps) {
  const [open, setOpen] = useState<ProposalRow | null>(null);
  const [version, setVersion] = useState(0);

  const load = useCallback(async () => {
    const rows = await apiClient.listProposals(workspaceId);
    // Pending first. The server sorts them so already.
    return { rows: [...rows].sort((a, b) => Number(b.status === "Pending") - Number(a.status === "Pending")) };
  }, [workspaceId]);

  const columns: DataTableColumn<ProposalRow>[] = [
    { id: "summary", header: "Summary", accessorKey: "summary" },
    { id: "app", header: "App", accessorKey: "app_slug" },
    { id: "by", header: "Proposed by", accessorKey: "proposed_by" },
    { id: "at", header: "When", cell: ({ row }) => row.original.created_at.replace("T", " ").replace("Z", "") },
    { id: "status", header: "Status", cell: ({ row }) => <StatusBadge tone={statusTone(row.original.status)} text={row.original.status} /> },
    {
      id: "checks",
      header: "Checks",
      cell: ({ row }) => (
        <span className="flex flex-wrap gap-1">
          {row.original.checks
            .filter((c) => c.outcome !== "pass")
            .map((c) => (
              <StatusBadge key={c.name} tone={tone(c)} text={CHECK_WORDS[c.name] ?? c.name} />
            ))}
          {row.original.checks.every((c) => c.outcome === "pass") && <StatusBadge tone="ok" text="All checks passed" />}
        </span>
      ),
    },
  ];

  return (
    <div className="space-y-4" data-testid="workspace-proposals">
      <PageHeader title="Proposals" description="App and rule changes waiting for a person to decide, and the ones already decided" />
      <DataTable
        key={`v${version}`}
        columns={columns}
        load={load}
        rowKey="id"
        onRowOpen={(row) => setOpen(row)}
      />
      <Drawer open={open !== null} onClose={() => setOpen(null)} title="Proposal">
        {open && (
          <ProposalDetail
            proposal={open}
            viewerEppn={viewerEppn}
            onPreview={onPreview}
            onTrial={onTrial}
            onDecided={() => {
              setOpen(null);
              setVersion((v) => v + 1);
            }}
          />
        )}
      </Drawer>
    </div>
  );
}

/** Why a decider is needed, who it is, and what would avoid it. Every sentence comes from the server. */
function WhoDecides({ authority: a }: { authority: NonNullable<ProposalRow["authority"]> }) {
  return (
    <section aria-labelledby="proposal-who" data-testid="who-decides">
      <h3 id="proposal-who" className="text-xs font-semibold uppercase tracking-wide text-gray-500">
        Who decides
      </h3>
      <p className="mt-2">{a.path_sentence}</p>
      {a.path === "approver" && a.no_approver && (
        <p className="mt-1 text-amber-700 dark:text-amber-400">
          Nobody can decide this yet. Ask {a.contact.length > 0 ? a.contact.join(", ") : "a Platform Admin"} to assign the position.
        </p>
      )}
      {a.path === "approver" && !a.no_approver && (
        <p className="mt-1">Can decide: {a.resolved.map((r) => r.display_name || r.eppn).join(", ")}.</p>
      )}
      <ul className="mt-2 list-disc pl-5 space-y-1">
        {a.reasons_detail.map((r, i) => (
          <li key={i}>
            {r.sentence}
            {r.remedy && <span className="block text-xs text-gray-500">To avoid this: {r.remedy}</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}

function ProposalDetail({
  proposal,
  viewerEppn,
  onDecided,
  onPreview,
  onTrial,
}: {
  proposal: ProposalRow;
  viewerEppn: string;
  onDecided: () => void;
  onPreview?: (proposal: ProposalRow) => void | Promise<void>;
  onTrial?: (trial: ProposalTrial, proposal: ProposalRow) => void | Promise<void>;
}) {
  const [mode, setMode] = useState<"approve" | "reject" | null>(null);
  const [trials, setTrials] = useState<{ limit: number; open: number } | null>(null);
  const [trying, setTrying] = useState(false);
  const [trialError, setTrialError] = useState<string | null>(null);
  const waiting = proposal.status === "Pending";
  useEffect(() => {
    if (!onTrial || !waiting) return;
    let live = true;
    apiClient
      .myTrials()
      .then((t) => live && setTrials({ limit: t.limit, open: t.trials.length }))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [onTrial, waiting]);

  const tryIt = async () => {
    if (!onTrial) return;
    setTrying(true);
    setTrialError(null);
    try {
      const trial = await apiClient.openTrial(proposal.id);
      await onTrial(trial, proposal);
    } catch (e) {
      setTrialError(e instanceof Error ? e.message : "The trial copy could not be made.");
    } finally {
      setTrying(false);
    }
  };
  const pending = proposal.status === "Pending";
  const needsSecondPerson = isFlagged(proposal) && proposal.proposed_by.toLowerCase() === viewerEppn.toLowerCase();

  return (
    <div className="space-y-5 text-sm" data-testid="proposal-detail">
      <div>
        <p className="font-medium text-gray-900 dark:text-gray-100">{proposal.summary}</p>
        <p className="text-xs text-gray-500">
          Proposed by {proposal.proposed_by} for {proposal.app_slug}. Status: {proposal.status}.
        </p>
      </div>

      <section aria-labelledby="proposal-changes">
        <h3 id="proposal-changes" className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          What changes
        </h3>
        <ul className="mt-2 list-disc pl-5 space-y-1">
          {proposal.changes.map((c, i) => (
            <li key={i}>{c}</li>
          ))}
        </ul>
      </section>

      {proposal.authority && <WhoDecides authority={proposal.authority} />}

      <section aria-labelledby="proposal-checks">
        <h3 id="proposal-checks" className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          Checks
        </h3>
        <ul className="mt-2 space-y-2">
          {proposal.checks.map((c) => (
            <li key={c.name} className="flex items-start gap-2">
              <StatusBadge tone={tone(c)} text={`${CHECK_WORDS[c.name] ?? c.name}: ${outcomeWord(c)}`} />
              <span className="text-xs text-gray-700 dark:text-gray-300">{c.messages.join(" ")}</span>
            </li>
          ))}
        </ul>
      </section>

      {pending && (
        <section className="space-y-3">
          {needsSecondPerson && (
            <p role="status" className="text-xs font-medium text-amber-700 dark:text-amber-300">
              A second person must approve this change.
            </p>
          )}
          {onTrial && trials && trials.open >= trials.limit && (
            <p className="text-xs text-gray-500">You have {trials.limit} trial copies open. Let one end, and you can make another.</p>
          )}
          {trialError && (
            <p role="alert" className="text-xs font-medium text-red-700 dark:text-red-300">
              {trialError}
            </p>
          )}
          <div className="flex gap-2">
            {onPreview && (
              <button
                type="button"
                onClick={() => void onPreview(proposal)}
                className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700"
              >
                Preview
              </button>
            )}
            {onTrial && trials && trials.open < trials.limit && (
              <button
                type="button"
                disabled={trying}
                onClick={() => void tryIt()}
                className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700 disabled:opacity-50"
              >
                Try it
              </button>
            )}
            <button
              type="button"
              disabled={needsSecondPerson}
              onClick={() => setMode("approve")}
              className="px-3 py-1.5 text-xs font-medium rounded bg-blue-600 text-white disabled:opacity-50 disabled:cursor-not-allowed"
            >
              Approve
            </button>
            <button
              type="button"
              onClick={() => setMode("reject")}
              className="px-3 py-1.5 text-xs font-medium rounded border border-gray-300 dark:border-gray-700"
            >
              Reject
            </button>
          </div>
          {mode && (
            <ConfirmAction
              verb={mode === "approve" ? "Confirm" : "Confirm"}
              target={mode === "approve" ? "approval" : "rejection"}
              consequence={
                mode === "approve"
                  ? "The change becomes live now, and the approval is recorded."
                  : "The change is not made. The proposal is closed."
              }
              onConfirm={async (reason) => {
                await apiClient.decideProposal(proposal.id, mode, reason, proposal.digest);
              }}
              onSuccess={onDecided}
            />
          )}
        </section>
      )}
    </div>
  );
}
