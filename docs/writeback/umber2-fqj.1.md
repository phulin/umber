# `umber2-fqj.1`: Borrowed multi-character source names

## Boundary and implementation

Canonical source tokenization now records byte boundaries while scanning an
untransformed multi-character control word and passes the resulting `&str`
slice of the current contiguous source backing directly to compact delivery.
The creating `get_token` path interns that borrow, while readonly `get_next`
probes the same borrow without adding a hash entry. Neither successful warmed
path constructs a semantic name.

If a successful `^^` reduction changes the logical characters relative to the
raw bytes, or an exact-byte name cannot be borrowed as UTF-8 text, scanning
switches once to the existing owned `ControlSequenceName` representation. The
fallback reconstructs any unchanged prefix and accumulates subsequent logical
character codes. Public `SourceToken` consumers still receive an owned name;
only the production compact projection consumes the call-local borrow.

No name cache, owner, table, heap indirection, lexer mode, compaction, or
retained lifetime was added. The borrowed spelling or owned fallback ends at
the existing tokenizer projection, before the packed `TokenWord` and direct
source provenance enter command delivery.

## Validation

The focused structural test proves literal `\alpha` uses the borrowed compact
projection while `\^^61lpha` uses the owned fallback with the same logical
name. Production identity coverage proves transformed and literal `\abc`
share one control-sequence identity through mutable and readonly delivery in
both exact-byte TeX82 and UnicodeExtended profiles.

Profiling-allocation tests warm source-delivery high water and the existing
`warmedname` hash entry, then measure 257 mutable deliveries and 257 readonly
deliveries independently. Both report zero allocation calls and zero requested
bytes in `DeliveryAndScan`.

`cargo test -q -p tex-command --tests`, `cargo test -q --tests`, and all four
gates in `scripts/check.sh` pass. No CPU profiling was run.
