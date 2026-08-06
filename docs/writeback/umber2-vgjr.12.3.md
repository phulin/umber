# umber2-vgjr.12.3 — implicit TeX82 default disposition

The hermetic TeX82 catalogue gate now validates the exact ordered `1..=1380`
module inventory, initializes every module from one typed
`ModuleDisposition::DeferredReview` value, and applies the existing sorted
shard overrides. Typed variants retain each resolved disposition, owner,
property ID, gap bead, and rationale. Shard property validation still enforces
the same unique canonical citations, semantic ownership, gap records, and exact
active Rust test links.

The focused gate pins the pre-migration resolved map as an ordered SHA-256
projection and pins its census at 946 reviewed / 434 deferred modules and 106
covered / 45 gap properties. The inventory sequence assertion and incomplete
inventory negative test make executable completeness replace the repeated base
records. Duplicate shard ranges, overlapping property sections, and conflicting
owners remain rejected.

The 11,047-line `tests/tex82-properties/dispositions.json` authority and its
generator branch were deleted. The surviving generator now emits only the
pinned module inventory. Catalogue, testing, fixture, and script documentation
describe the implicit typed default and the surviving regeneration path.

Validation was serialized on the final implementation tree: the focused six
catalogue tests passed under a 512 MiB cgroup after an uncapped `--no-run`
build; the complete native workspace passed under a 1 GiB cgroup after its
uncapped `--no-run` build; and `scripts/check.sh` passed all four gates under a
1 GiB cgroup. Before this writeback, the implementation comprised 245 additions
and 111 authored-line deletions, plus 11,047 deleted repetitive JSON lines. No
follow-up discovery was required.
