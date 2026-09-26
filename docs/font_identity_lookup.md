# Immutable font identity lookup

`FontStore` owns immutable font payloads in coarse append-only chunks. Dense
mutable font parameters remain separate. Those owners are intentional: a
generated font is published once and survives ordinary TeX operations, rather
than belonging to a scanner's temporary arena.

The generated-font lookup boundary must not build a font merely to discover
that the font already exists. Previously `try_expanded_font` cloned the source
name, parameters, and metrics, applied the expansion, and then scanned all live
fonts while recomputing their identities. The book profile attributes 3.18%
inclusive sampled cycles to `FontStore::by_source_identity`, overlapping 3.13%
in `LoadedFont::realized_identity`; metric projection and allocation are
additional work. These are overlapping observations, not additive savings.

The replacement computes an expanded font's realized identity directly from
the immutable source fields and proposed construction. It looks up that exact
identity before projecting or allocating metrics. Only a miss constructs and
publishes the generated font. The identity calculation is shared with ordinary
realized-font identity calculation, so lookup and publication cannot disagree
about which fields participate.

One derived index belongs to the same `FontStore` as the payloads. Its entries
map a realized identity to the first live matching font, preserving the old
ascending-slot search when independent copied fonts have equal identities.
Format restoration builds the index from validated immutable records. Clone,
checkpoint suffix removal, rejection, acceptance, and retained-prefix forks
preserve or restore entries alongside the logical font rows. No index entry
can resolve a discarded or replacement-incarnation font. The index is not
serialized, hashed as semantic state, or a second font payload owner.

The current public identity is `RealizedFontIdentity` and its accessor is
`realized_identity`. The former `FontSourceIdentity` alias and
`source_identity` forwarding accessor are removed; source ancestry still uses
realized identities and keeps its existing meaning and bytes. PDF resource
identity remains a separate, intentional equivalence relation.

Validation covers prospective versus constructed expanded identity, duplicate
identity first-match behavior, truncation and slot reuse, checkpoint rejection
and acceptance, retained-prefix lookup, and frozen-format restoration.
Performance measurements must use matched production binaries and identical
work endpoints; the native suite remains a semantic and ownership gate rather
than a wall-clock benchmark.
