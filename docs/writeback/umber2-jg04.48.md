# umber2-jg04.48: one INITEX current-string byte

The exhaustive canonical tracer is clean. After the merged e-TeX static pool
coordinates became exact, official two-phase e-TRIP first differed in the
final string-character capacity: the pinned oracle reported `13644`, while
Umber reported `13643`. Used strings, string capacity, used characters,
command projections, and normalized DVI were exact.

TeX82 §38 defines one current string between `str_start[str_ptr]` and
`pool_ptr`. Section 934 appends each exception word and language byte, then
terminates it with `make_string`; §1309 serializes the one aggregate
`pool_ptr`. Umber's INITEX projection accounted for one unfinished
current-string byte for every allocated exception. Compatibility TRIP has one
exception and hid the mistake, while e-TRIP's two exceptions overcounted one
byte.

The generic pool ledger now records whether that single unfinished byte is
already present and charges it only once across any number of exceptions. A
focused two-exception control proves both retained strings and their bytes are
counted while only one current-string byte exists. Persisted pool profile
version 10 and prepared-format producer contract 13 force reconstruction of
older images.

The entire e-TRIP string-pool block is now exact. The official gate advances
to an independent main-memory difference (`3317` expected, `1342` actual),
recorded observed-only as `umber2-jg04.49`. No comparison or normalization
changed.
