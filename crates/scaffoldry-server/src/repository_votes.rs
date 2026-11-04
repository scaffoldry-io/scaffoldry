//! Process v2 phase 4: a committee's votes, in `process_votes`.
//!
//! A vote, its count and, when the count closes the step, the decision are one transaction: it locks the instance row,
//! checks the step still waits, writes the vote, counts the votes of the voters fixed when the step was entered, and
//! closes the step if the rule says so. Two last votes at once close it once.

use crate::repository::{append_ledger_in_tx, process_event_in_tx, status_name, update_record_in_tx, AppendDecisionParams, PostgresRepository, RepositoryError, WriteCtx};
use scaffoldry_core::workflow::{vote_outcome, DecisionRule, RejectRule, Tally};
use scaffoldry_core::{ProcessInstance, ProcessStatus};
use serde_json::{json, Value};

/// What the step becomes if the votes close it one way.
pub struct Closing {
    pub next: ProcessInstance,
    pub record: Option<(String, String, Value, WriteCtx)>,
}

/// One vote and what closing the step would write.
pub struct Ballot {
    pub instance_id: String,
    pub step_id: String,
    pub voter: String,
    /// `approve` or `reject`.
    pub vote: String,
    pub comment: Option<String>,
    pub decision: DecisionRule,
    pub reject_rule: RejectRule,
    pub voters: Vec<String>,
    pub on_approve: Closing,
    pub on_reject: Closing,
    /// The ledger entry a close writes. Its payload gains the tally and the votes.
    pub ledger: AppendDecisionParams,
}

/// What a vote did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Voted {
    /// The vote is counted and the step stays open.
    Counted(Tally),
    /// The vote closed the step: approved or not.
    Closed { approved: bool, tally: Tally },
    /// The voter already cast this vote.
    Same,
    /// The step is no longer open.
    Stale,
}

/// A vote as the committee and the audit see it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoteRow {
    pub voter: String,
    pub vote: String,
    pub comment: Option<String>,
    pub at: String,
}

fn votes_in(tx: &mut postgres::Transaction<'_>, instance: &str, step: &str) -> Result<Vec<VoteRow>, RepositoryError> {
    Ok(tx
        .query("SELECT voter, vote, comment, at FROM process_votes WHERE instance_id = $1 AND step_id = $2 ORDER BY at, voter", &[&instance, &step])?
        .iter()
        .map(|r| VoteRow { voter: r.get(0), vote: r.get(1), comment: r.get(2), at: r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339() })
        .collect())
}

/// The tally of the votes cast by `voters`.
pub fn tally_of(votes: &[VoteRow], voters: &[String]) -> Tally {
    let counted = |kind: &str| votes.iter().filter(|v| v.vote == kind && voters.iter().any(|e| e.eq_ignore_ascii_case(&v.voter))).count();
    Tally { approve: counted("approve"), reject: counted("reject"), voters: voters.len() }
}

/// Forgets an instance's earlier votes when it starts again. The ledger keeps the votes of every decided step.
pub(crate) fn clear_votes_in_tx(tx: &mut postgres::Transaction<'_>, instance_id: &str) -> Result<(), RepositoryError> {
    tx.execute("DELETE FROM process_votes WHERE instance_id = $1", &[&instance_id])?;
    Ok(())
}

impl PostgresRepository {
    /// Casts `ballot` and closes the step if the votes now decide it.
    pub fn cast_vote(&self, ballot: Ballot) -> Result<Voted, RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT instance_json FROM process_instances WHERE id = $1 FOR UPDATE", &[&ballot.instance_id])? else {
                return Ok(Voted::Stale);
            };
            let stored: ProcessInstance = serde_json::from_value(row.get(0))?;
            if stored.status != ProcessStatus::Waiting || stored.waiting_step_id.as_deref() != Some(ballot.step_id.as_str()) {
                return Ok(Voted::Stale);
            }
            let before: Option<String> = tx
                .query_opt("SELECT vote FROM process_votes WHERE instance_id = $1 AND step_id = $2 AND lower(voter) = lower($3)", &[&ballot.instance_id, &ballot.step_id, &ballot.voter])?
                .map(|r| r.get(0));
            if before.as_deref() == Some(ballot.vote.as_str()) {
                return Ok(Voted::Same);
            }
            tx.execute(
                "INSERT INTO process_votes (instance_id, step_id, voter, vote, comment, at) VALUES ($1, $2, $3, $4, $5, NOW()) \
                 ON CONFLICT (instance_id, step_id, voter) DO UPDATE SET vote = EXCLUDED.vote, comment = EXCLUDED.comment, at = NOW()",
                &[&ballot.instance_id, &ballot.step_id, &ballot.voter, &ballot.vote, &ballot.comment],
            )?;
            process_event_in_tx(&mut tx, &stored, Some(&ballot.step_id), "voted", Some(&ballot.voter), &json!({ "vote": ballot.vote, "changed": before.is_some() }))?;
            let votes = votes_in(&mut tx, &ballot.instance_id, &ballot.step_id)?;
            let tally = tally_of(&votes, &ballot.voters);
            let Some(approved) = vote_outcome(ballot.decision, ballot.reject_rule, tally) else {
                tx.commit()?;
                return Ok(Voted::Counted(tally));
            };

            let closing = if approved { &ballot.on_approve } else { &ballot.on_reject };
            if let Some((slug, id, data, ctx)) = &closing.record {
                update_record_in_tx(&mut tx, slug, id, data, None, ctx)?;
            }
            let next = &closing.next;
            tx.execute(
                "UPDATE process_instances SET status = $2, instance_json = $3, updated_at = NOW() WHERE id = $1",
                &[&next.id, &status_name(&next.status), &serde_json::to_value(next)?],
            )?;
            let decision = if approved { "approve" } else { "reject" };
            let counted: Vec<Value> = votes
                .iter()
                .filter(|v| ballot.voters.iter().any(|e| e.eq_ignore_ascii_case(&v.voter)))
                .map(|v| json!({ "voter": v.voter, "vote": v.vote, "at": v.at }))
                .collect();
            process_event_in_tx(&mut tx, next, Some(&ballot.step_id), "decided", Some(&ballot.voter), &json!({ "decision": decision, "tally": tally }))?;
            process_event_in_tx(&mut tx, next, Some(&ballot.step_id), "step_left", Some(&ballot.voter), &json!({}))?;
            process_event_in_tx(&mut tx, next, None, "completed", Some(&ballot.voter), &json!({ "status": status_name(&next.status) }))?;
            crate::repository_timers::cancel_timers_in_tx(&mut tx, &next.id)?;
            crate::repository_task_grants::close_task_grants_in_tx(&mut tx, &next.id)?;
            crate::repository_work::close_work_in_tx(&mut tx, &next.id, "decided")?;
            let mut ledger = ballot.ledger;
            ledger.payload = json!({ "instance_id": next.id, "decision": decision, "tally": tally, "votes": counted });
            ledger.rationale = format!("{decision} by committee vote, {} for and {} against of {}, on {}", tally.approve, tally.reject, tally.voters, next.id);
            append_ledger_in_tx(&mut tx, ledger)?;
            tx.commit()?;
            Ok(Voted::Closed { approved, tally })
        })
    }

    /// The votes cast on an instance's committee step, oldest first.
    pub fn instance_votes(&self, instance_id: &str) -> Result<Vec<VoteRow>, RepositoryError> {
        let i = instance_id.to_string();
        self.with_client(move |c| {
            Ok(c.query("SELECT voter, vote, comment, at FROM process_votes WHERE instance_id = $1 ORDER BY at, voter", &[&i])?
                .iter()
                .map(|r| VoteRow { voter: r.get(0), vote: r.get(1), comment: r.get(2), at: r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339() })
                .collect())
        })
    }
}
