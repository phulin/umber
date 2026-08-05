# Incremental paragraph memoization deletion

Umber does not retain or replay paragraph input transactions, finished-line
graphs, paragraph mutation transitions, or paragraph-local provenance across
editor revisions. The former design is retained only in Git history and in the
measurement receipt
[`paragraph_replay_deletion_baseline.md`](paragraph_replay_deletion_baseline.md).

## Current restart contract

An edit maps accepted named boundaries into the new revision. The candidate
selects the nearest eligible accepted boundary before invalidated content,
restores its `tex-command`-owned `CommandSummary` and aggregate execution roots,
and resumes ordinary command processing. If no checkpoint is eligible, or an
external-input observation changed, execution starts at `JobStart`.

Paragraph construction, dependency observation, line breaking, page building,
effects, and shipout then execute normally. A candidate may adopt an accepted
suffix only after the generic canonical-state and boundary-schedule convergence
checks succeed. No paragraph-specific validation or mount path participates.

## Preserved reuse

The deletion does not change:

- token/input continuation restoration at accepted named boundaries;
- immutable editor fragment identities and revision mapping;
- accepted page/artifact prefix retention and generic suffix convergence;
- detached output provenance recipes for committed artifacts; or
- bounded pure caches for pretolerance, page breaking, and shipout.

Correctness remains byte-identical final artifacts and effects versus a cold
execution of the same revision and resource snapshot. Reuse is optional when a
boundary, resource dependency, schedule, or state identity does not validate.

## Non-goals

Do not reintroduce a paragraph replay feature flag, compatibility adapter,
retained node mount, dormant recorder, rerun-only mode, or persistent-arena
replacement. A future incremental strategy must be designed as a new generic
execution boundary and measured against the committed edit-restart workloads.
