# umber2-vgjr.9.2 — generated primitive catalogue views

Authority: [`tex-command`'s generated primitive views](../../crates/tex-command/src/primitives/generated.rs) and [parameter views](../../crates/tex-command/src/primitives/parameters.rs), based on source snapshot `dcfa38478`.

The exhaustive enum-backed declaration now projects stable operand-to-meaning
maps, exact ordered TeX82/e-TeX/LaTeX-compatibility/pdfTeX layer slices,
fresh-INITEX and format-registry policies, canonical observation identities,
the `max_non_prefixed_command` prefix partition, and deterministic Markdown
documentation tables. Registration and observation consume those views;
execution dispatch remains handwritten.

The parameter view covers all 103 TeX82, 11 e-TeX, and 56 pdfTeX parameter
spellings. It preserves bank cells, meanings, installation policy, job-clock
defaults, zero glue and token defaults, every pinned nonzero pdfTeX default,
and the two spellings sharing `PDF_MINOR_VERSION`. LaTeX compatibility adds no
parameter layer of its own.

Completeness tests bind all 86 expandable and 262 unexpandable enum variants
to unique stable operands. For every layer and both installation modes, the
generated and predecessor loops have identical registered meanings, live
meanings, and frozen-token indices, proving value and order identity. Separate
tests compare TeX82, e-TeX, and pdfTeX parameter rows directly with the
predecessor inventories and defaults, and format-load tests prove shadowed
live meanings remain untouched.

`umber2-vgjr.9.3` retains ownership of migrating the remaining external
parameter and policy consumers and deleting the predecessor inventories.

Validation was serialized as required:

- the focused `tex-command`, `tex-exec`, and `umber` scopes were compiled
  uncapped with `--no-run`, then their generated-view, profile, INITEX, and
  format-load tests passed under the 512 MiB cgroup;
- `cargo test -q --tests --no-run` completed uncapped, then the full routine
  suite passed under the 1 GiB cgroup; and
- `scripts/check.sh` passed all four gates under the 1 GiB cgroup, with both
  lint passes clean across 28 workspace members.

Implementation metrics are 1,157 additions and 34 deletions. Of those, 557
lines are production generated-view code, 272 are focused completeness/parity
tests in `tex-command`, and the remainder is migration wiring, cross-crate
predecessor parity proof, guidance, and this substantive writeback.
