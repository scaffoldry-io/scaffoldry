# Data layer — scenarios to walk

Fill in five real cases from your organization. One page is enough. Plain words are best. Use no real names or values.

The goal is to test the model in [data-layer.md](data-layer.md) against how people actually share and extend data. If most cases fit, the design is right. If they do not, we change the design before we build it.

## How to fill one in

Copy the block below once for each case.

```
Case:            A short name.
What it is:      One sentence. What data, and what is it for?

Who owns it
  Function:      Which function owns the data? (registrar, faculty affairs, IT, ...)
  Unit:          Which unit does it belong to?
  Steward:       Which position looks after it today?

Where it comes from
  Source:        A system, a spreadsheet, or typed in by people?
  How often it changes:

Who needs to see it
  Inside the unit:     Which positions or roles?
  Outside the unit:    Which units or positions, and why?
  How they get it now: Email, a shared sheet, a report, a login?

What the local unit adds
  New fields:    Which, and who maintains them?
  New records:   Anything the central data does not have?
  Does the central owner need to see these back?   Yes / No / Sometimes

How sensitive
  Which fields are sensitive, and what rule applies? (FERPA, PHI, HR, ...)
  Does the sensitivity change by who is asking?

What goes wrong today
  A copy that drifts. A request that waits. Someone sees too much. Someone cannot see enough.

How it should work
  In one or two sentences, in your own words.
```

## One worked example

I wrote this one to show the level of detail. It is **not** a real case, and it is not evidence about your institution.

```
Case:            New faculty onboarding.
What it is:      The record of a new faculty member, from offer to first term.

Who owns it
  Function:      Faculty affairs for rank, track, and tenure dates.
                 Space management for the office. IT for the computer.
  Unit:          The hiring department.
  Steward:       Each function has its own steward position.

Where it comes from
  Source:        The HR system for the person. The rest is typed in by each function.
  How often it changes:  Weekly during hiring season.

Who needs to see it
  Inside the unit:     The chair, the department administrator.
  Outside the unit:    The dean's office, for budget. The provost, for tenure review.
  How they get it now: A shared spreadsheet that is emailed.

What the local unit adds
  New fields:    Start-up funds and lab needs. The department maintains them.
  New records:   None.
  Does the central owner need to see these back?   Sometimes. The dean's office does.

How sensitive
  Tenure review dates and offer terms are restricted to faculty affairs and the chair.
  Office and computer details are not sensitive.

What goes wrong today
  The spreadsheet drifts. People outside the unit see offer terms by accident.

How it should work
  Each function keeps its own part. The chair sees all of it for their department.
  The dean's office sees the start-up funds, and only for their college.
```

In the model this case becomes three datasets owned by three functions and keyed by the person, plus a sidecar for the department's start-up funds. The chair's grant covers all three for their unit. The dean's grant covers the sidecar for the college. A reader will see the whole thing only by joining them in an app. If your real cases do not look like this, say so. That is the point.

## What I will do with your cases

For each one I will write the access and ownership it implies as a test, in plain words first. Then I will check each against the rules:

1. Who may read each field of each row, and why?
2. Who may write?
3. What does the central owner see back?
4. What would a steward need to do to change it, and who would be told?

Cases that need a rule the design does not have go on a list. We decide on those one at a time.

## Five kinds of case worth covering

Pick the ones that match your organization. If you have two of one kind, take both.

1. **A central list that units extend.** A unit wants extra columns on a list someone else owns.
2. **A record that two units both care about.** A joint appointment, a shared program, or a cross-listed course.
3. **A sensitive record with a narrow audience.** Student or health information that a few roles may see.
4. **A one-off share.** Someone needs one record or a few, for a reason, for a while.
5. **A copy that already exists.** A spreadsheet or extract that people keep and that the data layer should replace. This is the path around governance, so it tells us the most.
