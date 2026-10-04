---
name: least-software
description: Enforces the laziest, simplest, and most minimal solution that completely solves the problem. Reaches for standard library and native container capabilities before adding dependencies; questions unnecessary code (YAGNI).
---

# Least software

## The rule

The most reliable code is the code that was never written. Every added line is a liability to be maintained, reviewed, upgraded, and audited. Stop at the first layer that holds. Question whether a requested feature or abstraction needs to exist at all.

## The ladder

When designing or writing any implementation:
1. **YAGNI:** Does this problem need to be solved right now? If no, do not build it.
2. **Native Platform:** Can the operating system, container runtime, or existing framework solve this without code? (e.g. use standard environment variables, standard container mounts, or existing CLI tools).
3. **Standard Library:** Can the language's standard library do this before adding a third-party crate or npm package?
4. **Existing Dependencies:** If a library is required, use one already declared in the repository before adding a new one.
5. **Least Code:** One clear line is always better than fifty lines of bespoke abstraction.

## What must never happen

- Adding an external npm package or Cargo crate for a task achievable with 10 lines of standard library code
- Writing custom daemon loops, hypervisor layers, or token scrapers when standard container compose or native APIs exist
- Premature abstraction, generic factories, or speculative interfaces for features not yet required
- Introducing configuration options that nobody asked for
