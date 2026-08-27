# `umber2-66p0.38`: displayed error-context projection

Authority: TeX82 §§310--318 and e-TeX 2.6's extended source-name partition.

## Adopted boundary

Error-context selection now belongs to the command input owner while it walks
the live stack from the current level toward TeX's `bottom_line`. The walk
pseudoprints the current level and the nonnegative `\errorcontextlines`
budget directly into the final output. After that prefix is full, it retains
only the newest eligible stack position, which is the possible bottom level;
older omitted levels are never projected into strings.

The traversal still omits an exhausted non-current backed-up list, passes
through `\scantokens` pseudo-files, and stops at the first real file or stack
bottom. Negative budgets suppress the elision marker, zero and positive
budgets preserve TeX's display count, and a current level that is also the
bottom is rendered once. `tex-state` retains only §§316--318's two-line
pseudoprint kernel for one already-selected level.

The eager `Vec<ErrorContextLevel>` and its separate selection traversal are
deleted. The default path gains no cache, fast path, persistent owner, heap
indirection, compaction, or lifetime mechanism.

## Evidence

`error_context_selection_matches_tex310_omission_matrix` exhaustively compares
stack depths 0 through 8 and `\errorcontextlines` values -3 through 6 with a
literal bounded implementation of §310. Existing exact diagnostic fixtures
cover live and exhausted backed-up levels, nested `\scantokens`, real-file
framing, terminal fallback, and pseudoprint width cropping.

The affected `tex-command`, `tex-state`, and `tex-exec` suites pass, as does
the complete `cargo test -q --tests` routine suite. `scripts/check.sh` passes
all four gates. The authenticated arXiv `2606.12566` 20M run stops at status 1
with unchanged work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`, empty standard output,
no partial PDF, and the same normalized diagnostic SHA-256
`68031179ff7c37a0902ed1181ea753addeb0ea80ebc5f38881ed24fb40ac85b1`.
