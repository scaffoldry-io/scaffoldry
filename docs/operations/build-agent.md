# Build agent: setup and operation

An AI model runs on GitHub's hosted runners, implements one phase of `docs/plans/` at a time, and opens a pull request. CI checks it. A person merges. Nothing needs your own machine.

## How it works

1. You start a run: `gh workflow run build-agent.yml -f brief=guards -f phase=1`.
2. The **agent job** checks that the phase is the next open row of the run order and not blocked. It starts a database, installs the toolchains, and runs the model. The model writes tests, writes code, runs the checks, and commits **locally**. It has no push credentials and a read-only GitHub token.
3. The job hands back a git bundle and a short report.
4. The **publish job** has the write token and runs no model. It checks the bundle with `governance/scripts/agent-guard.py` taken from `main`, pushes the branch `agent/<brief>-phase-<n>`, opens the pull request, and starts CI on the branch.
5. CI runs on a GitHub runner. The same guard runs there too.
6. A person reviews and merges. If the chain is on, the merge starts the next phase. If repair is on, a failed CI asks the agent to fix it, three times at most.

## What it will not do

- Merge anything. A person always merges.
- Push to `main`.
- Change `.github/`, `.devcontainer/`, `AGENTS.md`, `LICENSE`, or an existing script in `governance/scripts/`.
- Add a dependency, unless the pull request carries the `deps-approved` label.
- Remove, skip, or ignore a test.
- Edit the plans beyond the Status cell of its own row and the status line of its brief.
- Start a phase that is out of order, already done, or marked "Needs ..." in the run order.

The guard enforces these. The model is also told, but the guard does not rely on that.

## One-time setup

These steps change repository settings and store a secret. Only you can do them. Workflows run from `main`, so first merge the pull request that adds these files.

```bash
# 1. Let workflows open pull requests. (They cannot merge. Do not let anything approve reviews.)
gh api -X PUT repos/scaffoldry-io/scaffoldry/actions/permissions/workflow \
  -f default_workflow_permissions=read -F can_approve_pull_request_reviews=true
```

```bash
# 2. Create the "agent" environment and limit it to main, so a branch cannot change the workflow and reach the key.
gh api -X PUT repos/scaffoldry-io/scaffoldry/environments/agent \
  -F deployment_branch_policy[protected_branches]=false -F deployment_branch_policy[custom_branch_policies]=true
gh api -X POST repos/scaffoldry-io/scaffoldry/environments/agent/deployment-branch-policies -f name=main
```

```bash
# 3. Store the API key in that environment. Paste it at the prompt. Never put it in a file or a chat.
gh secret set ANTHROPIC_API_KEY --env agent --repo scaffoldry-io/scaffoldry
```

Also set a monthly spend limit for that key in the Anthropic console. It is the real cost ceiling.

```bash
# 4. Variables. Start with the agent on and the automation off.
gh variable set AGENT_ENABLED --body true --repo scaffoldry-io/scaffoldry
gh variable set AGENT_AUTOCHAIN --body false --repo scaffoldry-io/scaffoldry
gh variable set AGENT_AUTOREPAIR --body false --repo scaffoldry-io/scaffoldry
gh variable set AGENT_MAX_RUNS_PER_DAY --body 6 --repo scaffoldry-io/scaffoldry
# Optional. The default model is claude-haiku-5-5 and the default turn limit is 80.
# gh variable set AGENT_MODEL --body claude-haiku-5-5 --repo scaffoldry-io/scaffoldry
# gh variable set AGENT_MAX_TURNS --body 80 --repo scaffoldry-io/scaffoldry
```

```bash
# 5. A label for dependency changes you have approved.
gh label create deps-approved --color 0e8a16 --description "A person approved the dependency change" --repo scaffoldry-io/scaffoldry
```

Recommended, and your call: require one approving review on `main` and apply protection to administrators. `AGENTS.md` says "reviewed pull request", and today protection requires none. With that setting on, an agent pull request cannot merge without a person.

## First run

```bash
gh workflow run build-agent.yml -f brief=guards -f phase=1 --repo scaffoldry-io/scaffoldry
gh run watch --repo scaffoldry-io/scaffoldry
```

Read the job summary for the agent's report. Review the pull request as you would any other. Turn on `AGENT_AUTOREPAIR` and then `AGENT_AUTOCHAIN` only after two or three good runs.

## Switches

| To do this | Run |
| --- | --- |
| Stop all agent runs | `gh variable set AGENT_ENABLED --body false` |
| Stop the chain only | `gh variable set AGENT_AUTOCHAIN --body false` |
| Stop repair only | `gh variable set AGENT_AUTOREPAIR --body false` |
| Switch the workflow off | `gh workflow disable build-agent.yml` |
| Stop a run in progress | `gh run cancel <run id>` |
| Retry a phase after deleting its branch | `git push origin --delete agent/<brief>-phase-<n>`, then run again |
| Ask for a fix by hand | `gh workflow run build-agent.yml -f brief=<b> -f phase=<n> -f mode=fix -f pr=<number>` |
| Run a phase out of order | add `-f allow_out_of_order=true` |
| Use a stronger model for one run | add `-f model=<model id>` |

## Safety design

- **Who can start it.** Only people with write access can dispatch a workflow. Nothing starts it from an issue, a comment, or a fork. A stranger cannot reach the prompt.
- **What the model reads.** Its prompt is fixed text plus a validated brief name and phase. No issue, comment, or pull request text is ever put into it. The one source of outside text is the repository itself, so the trust boundary is write access to the repository.
- **Where the secrets are.** The API key is an environment secret, available only to the job that runs the model and only to its model step. It is not available to forks or to branches.
- **What the model can do.** Its token is read-only and has no push credentials. A fixed list of tools is allowed: file edits and the build, test, and audit commands. Web tools are denied.
- **What is checked before anything is pushed.** The publish job runs the guard from `main`, not the agent's copy.
- **A person merges.** Branch protection requires the `validate` check, and merging is a human act.

## Limits and honest notes

- **Model capability.** The default is the smallest model, chosen for cost. The harder phases, such as the row and column rule engine, the calculation graph, and links with rollups, may exceed it. When a run fails or produces weak work, rerun that phase with `-f model=` set to a stronger model. CI is what decides quality, not the model's confidence.
- **Cost.** Each run is bounded by 80 turns and 90 minutes. The daily cap applies to the chain. The Anthropic spend limit is the hard stop.
- **Why CI is dispatched.** GitHub does not start workflows from events made with the workflow token. The publish job therefore starts CI on the branch with `workflow_dispatch`. The result attaches to the commit and should show as the `validate` check on the pull request. **Confirm this on the first run.** If it does not show, the fix is a GitHub App token or a fine-grained token for the publish job, not a change to branch protection.
- **Things to confirm on the first run.** That the action works with a read-only token in a dispatched run. That the environment scrub of secrets from the model's subprocesses applies without the `allowed_non_write_users` setting. If either fails, the run says so, and nothing is pushed.
- **Phases that need a person.** A phase marked "Needs ..." in the run order (a dependency decision, a file only you hold, a specification) is never started automatically. The chain opens an issue named "Build agent needs a person" and stops.
- **CI speed.** A full run was about two minutes when last measured. The planned test suites are larger. Add build caching when the Rust step passes about five minutes.
