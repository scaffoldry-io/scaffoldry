---
name: repository-practice
description: Use when committing, branching, merging, reviewing, or publishing work in a Scaffoldry repository, when setting up a repository or remote, or when an agent session is about to write to version control.
---

# Repository practice

## Overview

Plain words throughout, per the framework's fifth entry rule. This file
names roles, never people. Which person holds which authority is
answered by the organization's own register, kept in its own record,
never here.

The main line is what has been approved. Everything else is a proposal.

**Where this applies.** Three kinds of repository run this practice,
and the difference matters:

- **The template**, which is the product. Nothing reaches its main line
  without the full gate, because every organization adopts from it.
- **An organization's record**, created from the template, holding that
  organization's answers, handbook, and decisions.
- **A project repository**, holding one piece of work with the parts of
  the framework it needs, and the gates always.

A decision is recorded where the authority to make it sits, which the
register says, and never simply where the files are. So a proposal can
live in one repository and be decided in another, which is the ordinary
case for an organization deciding about its own projects.

**Core principle: nothing reaches the main line without a review it did
not write itself.**

**Deciding and carrying out are separate acts**, and whether one person
does both is an organization's choice rather than a rule. The approval
is recorded first and stands on its own, so a change that was approved
and could not be put into effect is a failure to carry it out, not a
failure to approve. Where a decision surface joins them, it joins them
for the person and keeps them separate in the record.

## Branches

- **Never implement on main.** Work happens on a branch named for its
  subject: `chapter/<name>` for a chapter, `rule/<name>` for a rule,
  `proposal/<name>` for anything else put up for decision,
  `skill/<name>` for a practice, `fix/<slug>` for a repair.
  (From subagent-driven-development's setup rule.)
- Parallel agent sessions use worktrees, one per branch, so two writers
  never share a tree. (From using-git-worktrees.)
- A branch is short-lived: it lands or it dies. Whoever runs the drift
  checker also lists branches older than a week and surfaces each to the owning change authority as a decision item; stale branches are escalated, never
  silently deleted.
- **Genesis exception:** a repository's first scaffolding commits, made
  before this practice's machinery exists in it, may land on the default
  branch. The exception ends the moment this skill is present in the
  tree.

## Commits

- Commit at step boundaries, small and frequent, each leaving the tree
  consistent. (From writing-plans granularity.)
- The first line states the why in plain prose; the body carries what a
  future reader needs and nothing they can get from the diff. House
  style, which governs all prose produced under this practice, commit
  messages included: no em-dashes.
- Agent-authored commits carry the Co-Authored-By trailer naming the
  model. Provenance is not decoration; it is who to ask.

## The merge gate

Every branch passes review before merging, and the review is
fresh-context, never the author's own session re-reading itself:

1. **Design artifacts** (skills, chapters, specs): an adversarial
   independent review per doubt-driven-development. The reviewer gets the work and the requirements, never the author's
   conclusions, because handing over a conclusion invites agreement.
2. **Code**: a review that answers two questions separately, does it do what was asked, and is it well built per
   subagent-driven-development, spec compliance and quality, against a
   diff file, treating what the author says about their own work as claims to check, not facts.
3. **Merges are always `git merge --no-fast-forward`**, so a merge commit exists
   for the record to live in. A fast-forward leaves the change authority nothing
   to read, and the change authority reads merge commits.
4. **Evidence named at the merge.** The merge commit names what was run
   and what it showed: the eval, the checker, the test command and
   output, for every check that exists for this artifact type. Where no
   check yet exists (a first chapter, a prose artifact with no eval),
   the merge commit says so plainly rather than staying silent, because
   an empty evidence line and an omitted one must not look alike. (From
   verification-before-completion, and the measurement skill's
   emptiness rule.)
5. **Review findings are weighed, not simply obeyed.** Sort each one: the requirements were unclear, it is a real problem to
   fix, it is real but not worth the cost of fixing, or the reviewer
   lacked context and it is not a problem. Decisions are written into the merge so none of them dies in a
   chat window.
6. **Human review is risk-tiered: Ship, Show, Ask.** Reference
   practice: Rouan Wilsenach's Ship/Show/Ask
   (martinfowler.com/articles/ship-show-ask.html), the industry frame
   for matching review to risk; ITIL's pre-authorized standard-change
   concept is the same idea in operations clothing. Adapted here for a
   practice whose committers are agents: the fresh-context agent review
   above is the FLOOR for every tier including Ship, because agent
   review is cheap and the scarce resource is change-authority attention. The
   tiers decide where the change authority's attention lands.

   **Ship**, merge after agent review, change authority sees the log: changes
   that are reversible, inward-facing, and substance-free. Typo and
   wording fixes in prose, ledger bookkeeping, upstream mirror refresh
   records, eval scenarios added without activating anything.

   **Show**, merge after agent review, then notify the change authority with the
   package for retro-review: ordinary skills and chapters, reference
   code with tests, adopted upstream drift with a pin move. Show's
   precondition is reversibility: if reverting would be costly or
   embarrassing, it is not Show. The change authority can demote any Show merge
   to a revert with one word.

   **Ask**, park until the change authority approves, never merged unattended:
   any change that is one or more of the following, and when the
   classification is uncertain it is Ask, the default moves up, never
   down.
   - Changes what exists rather than how it looks: identity, VISION,
     entry rules, any doctrine. **Any edit to this file, one character
     included, is Ask. Full stop.** This file defines the gate itself, and a small mistake here weakens
     everything the gate protects.
   - Licence posture, adoption of a NEW vendored source, or anything
     touching what may be distributed. A pin move on an existing source
     is Show only when the drift diff was read and preserves behaviour;
     drift that changes behaviour or adds capability is Ask, and pin
     moves never share a commit with refresh bookkeeping.
   - Public surface: remotes, publication, anything leaving the machine
   - Security-sensitive, in process OR content: permissions, secrets
     handling, the merge gate itself, and reference or product code
     whose diff touches authentication, input handling, or outbound
     network calls, wherever the file lives
   - Irreversible or destructive: deletions of substance, history
     rewrites, migrations
   Ask approval is an artifact, not an assertion: the merge commit
   carries `Approved-by: <change authority>` with the date, and the approval
   named this branch.

7. **Classification is recorded, enforced, and wrong classifications
   are reverted first.** Every merge commit carries one line: `Risk:
   ship|show|ask, because <the rubric line that decided it>`. The
   commit-msg hook in `reference/hooks/` enforces mechanically what a
   hook can reach: merge commits must carry the Risk line, and merges
   touching the protected paths (this file, README, licence files, the
   sources ledger) must be `Risk: ask` with an `Approved-by:` trailer.
   Install it in every clone: `cp reference/hooks/commit-msg
   .git/hooks/ && chmod +x .git/hooks/commit-msg`. Honestly stated: a
   hook enforces paths, not content, and `--no-verify` bypasses it, so
   the hook is a floor against error, not a wall against a determined
   session; content-based risks remain judgment, which is why
   uncertainty defaults up. A change found merged below its tier is
   REVERTED first and recorded second, never logged-and-kept; if an
   irreversible change lands unapproved, work stops and the change authority is
   told immediately.

8. **Delegation is written, never assumed.** Moving a class of change
   between weights is itself a significant edit to this file. One approval
   never generalises.
9. **One merger at a time.** Only the session holding the main worktree
   merges. A concurrent session whose Ship or Show branch clears review
   HANDS IT OFF with a note carrying the branch name, its risk tier,
   and its review state; the merger integrates in arrival order,
   resolves nothing silently, and re-runs the fast checks after each
   integration. Handoff and parking are different words on purpose:
   parked means waiting for the change authority, and an Ask branch is never
   handed off, never in a merge queue, and never merged by any session
   until the change authority's approval exists as an artifact.
10. **Conflicts re-enter review.** A branch that conflicts with main is
   rebased on its own branch, and because the diff changed, the
   resolution goes back through a scoped review before merging.
   Conflict resolution directly on main is implementation on main.

## Remotes and visibility

- Private by default. These repositories are working papers and
  unreleased product.
- **Remotes belong to the organization, so creating one is the
  public-surface authority's call**, private ones included; an agent
  proposes, that authority creates or approves.
- **Publication is distribution.** Making any repository public is a decision of the public-surface
  change authority, and it takes two gates, not one: the three entry
  rules from this repository's README.md (licence readable without
  counsel, vendor survives procurement review, nothing between client
  and system), and a separate publication review for what the entry
  rules never checked: internal-only material, client identifiers,
  strategy notes. Content fit to exist here is not automatically fit to
  publish.
- Once a server holds the record, the weights become mechanical:
  significant paths (doctrine, licences, entry rules, anything touching
  approval) require the named authority's signature; notable and
  routine paths need the review recorded but no signature; nobody
  pushes to the main line directly; and the checks run on every
  proposal, where a failure cannot merge.
- No secrets in any commit, ever; a committed secret is rotated, not
  deleted. (From security-and-hardening.)

## Provenance duties

- Outside sources move to a new version only with a note in
  SKILLS-SOURCES.md saying what changed and who decided;
  `reference/refresh-upstream.sh` watches for changes and never applies
  them.
- Where a part came from is recorded by the graft that brought it in,
  and written nowhere else, so it cannot drift from the truth.
- The earlier lab repository is closed. No new work lands there; it is
  kept as evidence that chapters cite, never copied from.

## A decision nobody acts on

A request for changes sat unread for hours while work carried on
elsewhere, because the decision surface tells the decider what is
waiting and tells the worker nothing. The person who made that decision
was entitled to assume it had landed.

So: **check what is waiting before starting work, and after any period
away.** That is the desk's opening screen, which the approved design
calls Today: what is waiting, what was said, and how long it has sat.

At a terminal, git shows a decision on its own, with no tool involved:

    git log -1 --notes=decisions --format=%N <proposal>

What git has no answer for is the question across all proposals at once,
because joining branches to their notes and filtering on a field is a
query, and git has no query language. That logic lives in the desk and
nowhere else. A standalone report was written, in shell and then in
Python, and both were deleted: the desk already reads the record and
shows decisions to people, and a second tool doing the same job is a
second thing to maintain and a second place for the two to disagree.

Answering a request for changes means saying so where it can be found:
the commit that answers it names the decision it answers, and the
proposal returns to the decider with the answer visible. A decision
recorded and then silently worked around is worse than one never made.

## Three layers, because one is not a control

A rule an agent is asked to follow is not a control. A control is
something it cannot do. This practice learned that the hard way: an
agent recorded an approval on a person's behalf from a chat message,
merged on the strength of the record it had just written, and every
check passed, because every check only asked whether the parts were
present, not whether a person had produced them.

So approvals rest on three layers, and only the middle one is real
enforcement:

**One, the local check.** The commit-msg hook in `reference/hooks/`.
It catches mistakes on the machine where work happens. It is a
courtesy, not a control: it can be deleted, skipped, or edited by
whoever works there, and the honest reason to keep it is that most
failures are mistakes, and catching them early is cheap.

Copying a repository does not bring its hooks, by design, so every
checkout installs the gate itself and nobody is told when they have
not. Run `reference/hooks/install.sh` to see whether the gate is
installed and whether what is installed still matches what is
recorded; `--install` puts it right. Not installed and drifted are
both findings, not silence. Worktrees of the same repository share one
set of hooks, so installing once covers them all.

**Two, the server check.** `reference/hooks/pre-receive`, installed on
the machine that holds the original. It runs before anything is
accepted, using the server's own copy of the script and its own copy of
the approvers list. Nothing anyone does to their own machine reaches
it. Two rules make it hold:

- **The approvers list lives on the server, outside the content.** If
  it travelled in the push, whoever pushes could add their own key in
  the same push and approve themselves. Where the organization has a
  staff directory, the list is provisioned from it rather than edited
  by hand.
- **Changing who may approve requires an approval from someone else.**
  The path holding approvers is protected, and no person's approval
  counts for the change that adds their own key.

**Three, the audit.** `reference/audit/audit-approvals.sh` walks the
main line and reports every change that arrived without a verifiable
approval. It runs on a schedule, from a machine that is not the one
being audited, and its report goes to the people named in the register.

**What none of this prevents, stated plainly.** Whoever controls the
server can change the server. There is no arrangement in which the
administrator cannot administer. What the three layers do is make that
the only remaining path, make it require deliberate action rather than
convenience, and make it visible afterwards to people who did not take
it. The record is append-only and lives in every copy, so a change
made outside the process disagrees with every other copy of history and
shows up in the next audit. Prevention stops everyone else; detection
is what covers the person prevention cannot. An organization that wants
more separates the roles: the person who administers the server is not
the person who approves changes, and neither can complete the other's
act alone.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "It's a tiny fix, straight to main is fine" | The size of the change is not the size of the risk. Small commits on branches are cheap; the habit of exceptions is not. |
| "I reviewed it myself carefully" | The author's session validating the author's work is the confirmation trap. Fresh context or it is not a review. |
| "CI will catch it later" | CI proves what it checks. The review gate exists for what CI cannot check: whether this was the right change. |
| "We need it public to share with one person" | A remote can stay private and still share. Public is distribution, and distribution is a approved decision. |
| "Branch protection slows the change authority down" | It slows everyone down identically, which is the point. The change authority approves; nobody bypasses, so approval means something. |

## Red Flags: STOP

- About to commit to main directly, any repository, any size
- A merge commit with no named evidence
- A push to a remote nobody decided to create
- Force-push to main, under any justification
- A reviewer prompt that includes the author's conclusions
- A secret in a diff, even momentarily

## Verification

Before merging any branch:

- [ ] The work happened on a branch, not main
- [ ] A independent review ran and its findings were reconciled with rulings
- [ ] The merge was `--no-fast-forward` and its commit names the verification evidence, or says plainly that no check exists yet for this artifact type
- [ ] The merge commit carries the Co-Authored-By trailer when an agent merged
- [ ] The change carries a weight, the merge commit carries the Risk line, and significant merges carry the Approved-by trailer naming the authority and date
- [ ] No secrets in the diff
