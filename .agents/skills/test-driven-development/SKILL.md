---
name: test-driven-development
description: Guides writing failing tests first before implementation code, verifying the test fails for the right reason, implementing minimal code to pass, and refactoring with test safety.
---

# Test-driven development

## The rule

No production code is written without a prior failing test that demonstrates the need for it. The test must fail when the feature is absent, fail for the expected reason, and pass when the implementation is complete.

## The workflow

1. **Write the test first:** Define expected behavior, inputs, and outputs in an automated test.
2. **Execute and observe failure:** Run the test suite and confirm that the test fails specifically because the capability does not yet exist.
3. **Write the minimal implementation:** Write the smallest amount of code required to make the test pass.
4. **Execute and observe pass:** Run the test suite and verify green status.
5. **Refactor:** Clean up duplication and tighten types while keeping the tests green.

## What must never happen

- Writing code and tests simultaneously in the same commit without proving the test ever failed
- Claiming a feature is functional without an automated test output proving it in the active session
- Disabling, skipping, or mocking out the core logic being verified
