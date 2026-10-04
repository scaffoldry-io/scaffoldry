---
name: systematic-debugging
description: Enforces disciplined, hypothesis-driven debugging. Eliminates trial-and-error guessing loops by creating minimal reproduction cases, identifying root causes, and verifying fixes empirically.
---

# Systematic debugging

## The rule

Never guess at a fix. When an error or unexpected behavior occurs, construct a minimal reproduction, identify the root cause through empirical tracing, and verify the resolution before proposing a commit.

## The debugging loop

1. **Capture the exact symptom:** Record the exact error message, exit code, and environment conditions.
2. **Reproduce minimally:** Isolate the smallest command, script, or test that consistently triggers the issue.
3. **Trace root cause:** Inspect the system boundary, process state, logs, or network stack. Formulate a falsifiable hypothesis.
4. **Apply targeted fix:** Modify only the component causing the root failure.
5. **Verify elimination:** Confirm the reproduction case passes and no secondary regressions are introduced.

## What must never happen

- Modifying code or configuration speculatively hoping an error goes away
- Retrying a failed command repeatedly without changing conditions or gathering new telemetry
- Silencing errors or catching exceptions without addressing the underlying condition
