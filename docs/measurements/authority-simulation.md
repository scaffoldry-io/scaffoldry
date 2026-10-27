# Decision authority simulation

Date: 2026-10-27. Run time for all twelve runs on the machine that wrote this: 1.55 seconds.

One position is a bottleneck under the matrices that send High changes to a single privacy officer. Under "High to the privacy officer" (everyone present), the privacy officer of unit 0 has a busiest week of 108 items. A delegation to a deputy, or a row at each college that sends High changes to the college's own privacy contact, would relieve it. 6 of the twelve runs have a week above 50 items.

This simulation does not measure waiting time, and it does not measure whether people find the screens clear. Waiting time needs real decisions: see `GET /admin/authority/load`.

Seed 20261027. 5000 people, 200 units in four levels (1 institution, 9 colleges, 40 departments, 150 programs), 400 workspaces with one to four owners each, 10000 proposals over 13 weeks. 70% of proposals raise no check, 20% raise one flagged check, 10% raise two or three. 30% of proposers are owners. People who hold a position are never drawn as owners or members. Eight platform admins can be the second person on any workspace.

Away is a delegation window, the only absence the resolver knows. The holder still resolves and the delegate is added, so a delegate adds a second approver and does not take the first one away. A vacant position resolves to no one at that unit, and the walk continues up.

Each approver's load is the number of proposals that resolve to that person in a week. A proposal that resolves to two people counts for both. The busiest column is the approver with the largest week.

## Built-in defaults

No rows and no signals. Every raised check is Moderate and needs a second owner or admin.

| Scenario | Direct | Second person | Approver | Applied by the proposer alone | No approver | Only the proposer | Approvers with items | Busiest approver | Median / 95th / largest week |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Everyone present | 70.4% | 29.6% | 0.0% | 20.7% | 0 | 0 | 0 | none | 0 / 0 / 0 |
| 5% of positions vacant | 70.4% | 29.6% | 0.0% | 20.7% | 0 | 0 | 0 | none | 0 / 0 / 0 |
| 10% of holders away, with a delegate, for two weeks | 70.4% | 29.6% | 0.0% | 20.7% | 0 | 0 | 0 | none | 0 / 0 / 0 |

## Moderate to the department chair

One row at the institution: a Moderate change needs the chair of the workspace's department, found by walking up from its unit.

| Scenario | Direct | Second person | Approver | Applied by the proposer alone | No approver | Only the proposer | Approvers with items | Busiest approver | Median / 95th / largest week |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Everyone present | 70.4% | 0.0% | 29.6% | 20.7% | 0 | 0 | 40 | chair of unit 34 | 11 / 18 / 18 |
| 5% of positions vacant | 70.4% | 0.0% | 29.6% | 20.7% | 54 | 0 | 39 | chair of unit 34 | 11 / 18 / 18 |
| 10% of holders away, with a delegate, for two weeks | 70.4% | 0.0% | 29.6% | 20.7% | 0 | 0 | 44 | chair of unit 34 | 11 / 18 / 18 |

## High to the privacy officer

Three checks (sensitivity, page_sensitivity, data_loss) count as High at the institution. A High change needs the privacy officer. Other raised checks stay Moderate and need a second person.

| Scenario | Direct | Second person | Approver | Applied by the proposer alone | No approver | Only the proposer | Approvers with items | Busiest approver | Median / 95th / largest week |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Everyone present | 70.4% | 18.4% | 11.2% | 20.7% | 0 | 0 | 1 | privacy officer of unit 0 | 87 / 108 / 108 |
| 5% of positions vacant | 70.4% | 18.4% | 11.2% | 20.7% | 0 | 0 | 1 | privacy officer of unit 0 | 87 / 108 / 108 |
| 10% of holders away, with a delegate, for two weeks | 70.4% | 18.4% | 11.2% | 20.7% | 0 | 0 | 1 | privacy officer of unit 0 | 87 / 108 / 108 |

## Both together

The two rows above and the three High signals.

| Scenario | Direct | Second person | Approver | Applied by the proposer alone | No approver | Only the proposer | Approvers with items | Busiest approver | Median / 95th / largest week |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Everyone present | 70.4% | 0.0% | 29.6% | 20.7% | 0 | 0 | 41 | privacy officer of unit 0 | 87 / 108 / 108 |
| 5% of positions vacant | 70.4% | 0.0% | 29.6% | 20.7% | 31 | 0 | 40 | privacy officer of unit 0 | 87 / 108 / 108 |
| 10% of holders away, with a delegate, for two weeks | 70.4% | 0.0% | 29.6% | 20.7% | 0 | 0 | 45 | privacy officer of unit 0 | 87 / 108 / 108 |

