---
name: measurement-discipline
description: Use when about to state a fact about a system, a platform capability, a licence, a count, or an external state, whether asserting it, planning around it, or reporting it onward. Use especially when the fact came from documentation, a handover report, a listing, or memory rather than from a check run fresh in this session.
---

# Measurement Discipline

## Overview

A claim about a system is measured, never believed. Documentation
describes intent, reports describe the past, memory describes neither,
and all three are routinely wrong in ways that read as confident.

**Core principle: reconcile what you counted against what you meant to
count.**

**Violating the letter of this discipline is violating its spirit.**

## The Iron Law

```
NO CLAIM ABOUT A SYSTEM WITHOUT A MEASUREMENT
THAT COULD HAVE COME BACK THE OTHER WAY
```

A check that cannot fail is not a check. If your verification would have
printed the same clean answer whether the claim were true or false, you
have not verified anything, you have decorated the claim.

## The Gate Function

```
BEFORE stating a fact about a system:

1. SOURCE: did this come from a measurement I ran, or from
   documentation, a report, a listing, or memory?
   - Not from a measurement: run one, or mark the claim unverified.
2. CONTROL: what would this check show if the claim were false?
   - Run the control FIRST where cheap. A result without a control
     that failed is an artefact until proven otherwise.
3. COVERAGE: did the check run over everything I meant to check?
   - Count what was found, state it against what was expected.
     "116 LICENSE plus 2 LICENSE.txt against 121 packages" catches
     in one line what "every LICENSE file checked" hides.
4. EMPTINESS: if the result is empty or clean, does the check say
   what it verified? Zero findings over zero items is not a clean
   bill, it is a broken check.
5. ONLY THEN: state the claim, with the evidence beside it.
```

## The Five Failure Shapes

Each of these was hit in one working day, in one codebase, by careful
sessions. The class is common, not exotic.

**1. The hollow check.** A fixture drifted from the shape its module
filters on, the module returned empty, and the test passed while
checking nothing. Variant: a test asserting the number a function
returns while its comment promises placement, check and promise two
lines apart.
*Counter: assert on the outcome promised, not the value returned, and
prove the fixture survives the filter before trusting the test.*

**2. The half-covered sweep.** A licence audit guarded on `[ -f
LICENSE ]` and never opened two files named LICENSE.txt. A bulk rename
ran `find -name '*.js' -o -name '*.jsx' -exec`, where the exec binds
only to the last branch, editing every .jsx and not one .js. Both
returned clean.
*Counter: enumerate name variants first, and report files-found against
population so a silent skip surfaces as a gap.*

**3. The believed state.** Two background agents were reported running
five times over an hour. They had been dead fifty-eight minutes,
killed by a foreground interrupt. The first actual check then misread
symlink sizes and concluded they had never started.
*Counter: state is measured from the artefact, transcript bytes, file
mtime through the symlink, process table, never from the memory of
having started something.*

**4. The presence mistaken for behaviour.** Record-level permission
scopes were accepted, stored, and reported as attached by the platform,
and filtered nothing. Endpoints existing in a listing proved nothing
about what they enforce.
*Counter: prove behaviour with a subject the rule should refuse, and
watch it refused. Binding is proven by the control failing first.*

**5. The measurement that should gate the feature.** A field type
change reported success while touching only metadata; the loss arrived
later, when a coerced empty value was written back. The honest dialog
could only be built because the measurement ran before the feature.
*Counter: when a feature's safety claim depends on platform behaviour,
the measurement is the first task on the card, not the last.*

## Anything a person will look at is measured by looking at it

Counting elements in generated output is not evidence that the output
is right. A page can contain every tag you searched for and still be
unreadable, unstyled, overlapping, cut off, or blank.

This is failure shape 4 in a different costume: the element is present,
and presence is not behaviour.

```
BEFORE claiming a page, document, diagram or screen works:

1. RENDER it the way a person will receive it, in a browser.
2. LOOK at it. A screenshot is the measurement.
3. MEASURE what looking cannot settle: did the diagram produce a
   drawing, what size, does it fit its container, does the page
   scroll sideways, are all the labels there.
4. STATE found against expected, as with any other coverage claim.
```

The control matters here as much as anywhere. Render something that
should fail, a document with a deliberately broken diagram, and confirm
it looks broken. A visual check that would have looked fine either way
has measured nothing.

**One state per page.** The control and the subject go in separate
renderings, not side by side. A page showing both is readable to the
person who built it and misleading to everybody else: the reviewer sees
the control's buttons and reasonably concludes the subject offers them
too. This happened. The check was right, the artefact was not, and the
person reading it was correct to object.

**Set the harness up the way the product runs.** The first visual check
of the desk's diagram support showed a giant blue icon and no text. The
change was fine; the harness had imported the module without the
startup step that loads the stylesheet. That is worth knowing before
reporting a defect that is not there, and it is also the reason to do
this at all: neither the fault nor its absence was visible from
counting tags.

**What this does not require.** Not every commit. This applies to
anything whose output a person reads on a screen: a page, a document
view, a diagram, an email, a printed report. A parser or an API client
is measured the usual way.

## Reporting Measured Facts

- State the claim with its evidence and its date. Measured facts decay;
  a dated measurement invites re-measurement, an undated one impersonates
  a permanent truth.
- Distinguish the three honest verdicts: verified true, verified false,
  and not verified. "Probably" is a plan to be surprised.
- When relaying another session's report, either verify the load-bearing
  claims or attribute them as unverified. Relaying is asserting.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "The documentation says so" | Documentation describes intent. The instance you run is the fact. |
| "The report said it was running" | The report was true when written, at best. State is measured now or not known. |
| "The check came back clean" | Clean over what? A check that ran over half the population returns clean on the other half every time. |
| "The endpoint exists, so it works" | Accepted, stored, and reported as attached, and it filtered nothing. Presence is not behaviour. |
| "Running a control doubles the work" | The control is the work. Without it you measured your own assumptions. |
| "It's a small claim, not worth measuring" | Small claims compound into plans. The five failure shapes above were all small claims. |
| "I measured it last week" | Then say so, with the date, as last week's fact. Systems change under you. |

## Red Flags: STOP and Measure

- About to write "should", "probably", or "I believe" about a system state
- Quoting a count you did not produce this session
- A sweep or audit whose result is clean and whose coverage you cannot state
- A test you cannot name the failing condition for
- Reporting a background process, agent, or job as running without a fresh artefact check
- Planning a feature on a platform behaviour nobody has exercised
- Relaying a handover report's claims as facts
- Claiming a page, screen or diagram works without having looked at it

**All of these mean: stop, run the measurement, state it with evidence.**

## Verification Checklist

Before a claim ships in a report, a card, or a commit message:

- [ ] The claim traces to a measurement run in this session, or carries its date and an unverified marker
- [ ] The measurement had a control, or its absence is named
- [ ] Coverage is stated as found-against-expected
- [ ] Empty results say what they verified
- [ ] The evidence is beside the claim, not in your memory of it
