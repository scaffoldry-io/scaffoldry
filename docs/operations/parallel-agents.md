# Running two agents at once

Two agents can work on the plans at the same time. They must never take the same phase, and neither may build on work the other has not merged.

The lock is a branch. A branch name can be created on GitHub only once. The first agent to push `feat/<brief>-phase-<n>` owns that phase.

## Any tool can take part

The agents do not need to share a session, a project, or a vendor. A claim is a branch on GitHub, so any agent that can run Python and `git push` can take part. Nothing here depends on one tool.

Each agent merges its own pull request when CI is green. If your tool cannot merge, say so in your final message so Johann can do it. Do not leave a green pull request waiting.

## The commands

Run these from the repository root. They use only Python and git.

| Command | What it does |
| --- | --- |
| `python3 governance/scripts/next-phase.py claims` | Lists the phases that are held right now |
| `python3 governance/scripts/next-phase.py available` | Prints the next phase you may take |
| `python3 governance/scripts/next-phase.py claim <brief> <n>` | Takes the phase. Prints the checkout command |
| `python3 governance/scripts/next-phase.py release <brief> <n>` | Gives it up when you stop without a pull request |

A claim is refused when:

- someone already holds that phase;
- someone holds any phase of the same brief. One agent per brief;
- an earlier phase of that brief is not COMPLETED;
- the row says `Needs`, or its phase is not a single number. Those need Johann.

`available` looks at the first four open rows. It never looks past a row that needs Johann, because later rows may depend on it.

## What the tool cannot see

It cannot see that a phase in one brief needs a phase in another brief. Each brief says so in its opening lines, for example "Phase 1 needs `foundation.md` phase 4".

Before you claim, read the opening lines of your brief. If it needs a phase that another agent holds, do not wait on it and do not start. Run `release`, and tell Johann.

## Each agent works alone

- **Own checkout.** Use a second clone or `git worktree add ../agent-b origin/main`. Never share a working tree.
- **Own database.** The test suite resets shared state. Give each agent its own database and point `DATABASE_URL` at it:

  ```
  createdb -h 127.0.0.1 -p 5433 -U scaffoldry scaffoldry_agent_b
  export DATABASE_URL=postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry_agent_b
  cargo test --workspace -- --test-threads=1
  ```

  If the agents run in separate containers, each already has its own database.
- **Migration numbers** come from the README. Do not pick your own.

## Finishing

1. Merge `origin/main` into your branch. Another agent's pull request may have landed.
2. Run every check again after the merge. Paste the output.
3. If `docs/plans/README.md` conflicts, keep both Status cells. Change only your own row.
4. Open the pull request to `main`. Merge it yourself when CI is green. Use a squash merge, as the history does.
5. Your claim ends when your row says COMPLETED. Delete the branch if you like. A leftover branch for a COMPLETED row holds nothing.

## When to stop and tell Johann

- A brief needs a dependency, a file, or a decision.
- The other agent holds a phase yours needs.
- A check fails in a phase marked COMPLETED.
- Your claim command is refused and you do not understand why.

One phase per agent per session. Do not start the next one.
