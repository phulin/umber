# `umber2-66p0.43`: borrowed `\ifx` meaning comparison

## End state

TeX82 §507's two raw `get_next` deliveries still write into two
caller-local `CurrentCommand` slots while scanner status is temporarily
`normal`. Those slots now remain the sole operand owners through the entire
comparison. `evaluate_ifx` borrows each `ResolvedMeaning` with `meaning_ref`;
the macro case compares flags and borrows parameter and replacement spans
directly through each existing `DefinitionRef`.

The former two owned `ResolvedMeaning` constructions, two cloned definition
owners, and two owning `DefinitionView` reconstructions are absent. No cache,
fast path, allocation, heap indirection, compaction, additional owner, or
lifetime mechanism replaced them.

## Deterministic deletion evidence

The architecture boundary test requires exactly two `meaning_ref` calls in
`evaluate_ifx`, rejects owned `meaning` calls, and rejects both `clone` and
`CommandContext::definition` from the comparison helper. The definition-arena
test borrows both token spans through a live id and proves that its semantic
owner count is unchanged. The semantic matrix separately covers equal content
at distinct allocation identities, unequal flags, unequal parameter text,
unequal replacement text, and undefined equality.

## Exact behavior and gates

The authenticated pinned arXiv control stopped intentionally at status 1 and
the exact fuel limit. It preserved the work vector
`(20000000,19913119,2218327,6020965,16785710,4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
deferred-write expansions. Standard output was empty and 124 authenticated
distribution resources were acquired from the pinned offline closure.

Focused `tex-state` definition-arena tests, focused `tex-command` `\ifx` and
architecture-boundary tests, and `cargo test -q --tests` passed.
`scripts/check.sh` reported all four gates passed.
