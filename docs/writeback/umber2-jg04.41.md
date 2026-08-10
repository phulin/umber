# umber2-jg04.41: TeX82 control-sequence accounting

The exhaustive canonical tracer is clean. After the string-pool and
main-memory rows became exact, fresh-cache compatibility TRIP first differed in
§1334's final control-sequence statistic: the pinned oracle reported `372`
while Umber reported `388`. Command, geometry, and normalized-DVI hashes were
already exact.

A pinned Web2C TeX82 hash dump proved the oracle begins the loaded run with
`cs_count=341` and ends with `372`. Comparing all final occupied spellings
showed Umber's exact sixteen extras: every fifteen null/one-character spellings
and the retained `FONT?` string used for an active-character font identifier.
No multiletter semantic control sequence was missing.

TeX82 §259 increments `cs_count` only when `id_lookup` allocates a hash entry,
and §256 never removes one. Sections 356 and 372 route null and one-character
spellings directly to fixed `eqtb` slots instead of `id_lookup`. Section 1252
constructs `FONT`/`FONT<char>` as retained string-pool values for null or active
font identifiers, not as control sequences. Section 1309 serializes the
occupied hash set, and §1334 reports that exact count.

The generic fix now marks hash occupancy only for multiletter names and gives
retained font-identifier strings an unhashed interning path. Focused controls
prove null, one-character, active, internal, and retained `FONT?` spellings do
not affect the count, while a multiletter name remains occupied across format
round-trip. Fresh-cache TRIP now matches `372` exactly and advances to the
independent input-stack statistic (`38` expected, `36` actual), recorded
observed-only as `umber2-jg04.42`.
