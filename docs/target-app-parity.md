# Target-app parity: the assessment

> **Status:** 2026-10-31 · first assessment (M0).

**Targets:** Jira (Software and Work Management, Cloud as the reference) and Confluence
(Cloud). TrackCraft tracks the features both share with their Data Center editions, but observes
Cloud, which is the free, current reference.

## Where we are

| Dimension | Weight | Now | Notes |
| --- | --- | --- | --- |
| Tracker core (issues, workflow, comments, links) | 20% | ~45% | no history, attachments, watchers, custom fields |
| Search (JQL, filters) | 10% | ~40% | no history operators, saved filters, custom fields |
| Agile (boards, backlog, sprints, reports) | 15% | ~10% | board view only |
| Wiki core (spaces, pages, versions) | 15% | ~40% | no comments, diff, templates |
| Editor (rich text) | 15% | ~5% | model only, no editing UI |
| UI | 10% | 0% | M1 |
| Collaboration (server, permissions, notifications) | 10% | 0% | M4 |
| Interop (REST, import/export) | 5% | ~5% | ADF shape only |
| **Weighted** | | **~20%** of the feature surface, **~5%** ready for real work | |

The catalog (`docs/parity-checklist.md`) is the measured counterpart; this table is the
judgement on depth. Both get re-assessed at each milestone.

## Methodology

Weights reflect how much of a typical team's week each dimension carries. "Now" is a judgement
of depth (not breadth) against observed product behaviour in `plan/atlassian/`.
