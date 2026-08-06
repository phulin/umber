# Fixture-layout migration retirement

Issue: `umber2-vgjr.18.3`

## Compatibility disposition

`fixturegen --migrate-layout` and `fixturegen --migrate-pdf-layout` are retired
compatibility commands. A repository-wide tracked-source search at base
`1a748d231dd23015d154cd242bf1c35f68c81a68` found no caller: the commands
appeared only in their fixturegen dispatch, usage text, implementation/tests,
and maintainer documentation. Ordinary regeneration, PDF publication, corpus
sync, reference-DVI publication, and classic BibTeX/command-semantic cohort
publication use separate active entry points.

The completed closed fixture directories are the sole surviving layout
authority. `fixture_transaction.rs` is the extracted shared publication owner;
`--cohort-transaction`, ordinary single-case publication, PDF cohorts, corpus
sync, and reference-DVI publication retain the same transaction schema, plan
digest, rollback, retained-root recovery, and post-commit garbage collection.

## Final current-tree receipt

Before removing either command, the base tree ran each read-only plan with the
freshly built fixturegen binary. The output is deterministic, consists of one
`path: files bytes sha256=...` row per closed case, and made no writes.

| Retired command               | Rows |  Bytes | SHA-256 of exact stdout                                            |
| ----------------------------- | ---: | -----: | ------------------------------------------------------------------ |
| `--migrate-layout --plan`     |  150 | 17,664 | `907c32468f80f56664c610029880dfee55a6b76d1b2160366b96ce0ca9a134db` |
| `--migrate-pdf-layout --plan` |   15 |  1,758 | `545dda4d7c7cef26a1fa04ec53bf0cd2528cbe1c7f08f39067641164f04fd22f` |

These whole-report identities preserve every per-case inventory digest without
retaining a second executable registry. The exact former implementations and
their report construction remain recoverable from the named base commit.

## Deletion accounting

Command dispatch, the two one-time registries, their migration-only planner,
and migration-only tests count as retired source. The atomic transaction,
closed-inventory helpers, classic-case sealing, ordinary-regeneration tests,
and rollback/recovery/garbage-collection tests moved to surviving owners and
are excluded from deletion credit. No fixture bytes or case membership changed.

The complete tracked change is 408 additions and 1,709 deletions, a 1,301-line
net reduction. Retirement credit is 954 production lines (690 removed while
extracting the transaction module, 250 from the PDF registry, and 14 net CLI
dispatch/usage lines) plus 492 migration-only test lines, or 1,446 authored
lines. The surviving 786-line transaction module and 155 source lines for the
two moved ordinary-regeneration tests are explicitly excluded. Documentation,
active-caller renames, and rustfmt-only churn in two otherwise untouched files
are also excluded from retirement credit.
