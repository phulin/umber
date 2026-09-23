# Browser prefetch policy ownership

The browser manifest resolver uses `umber-distribution::PrefetchPolicy` through
the `umber-wasm` DTO bindings for literal lookup hints, queueing, budgeted
selection, dependency closure, and replay escalation. Authored JavaScript owns
HTTP acquisition, verified object caching, run identity, and transport metrics.
It never scans TeX source or independently decides predictive candidates.

The catalog binding is required to validate a manifest and plan required
lookups. The prefetch binding is a separate optional capability for standalone
resolver clients that only need required lookups. A resolver constructed before
WASM initialization can bind the policy later, before `beginRun`; the compile
facade does this after loading the module. If the policy is absent, the
resolver returns no startup, runtime, or replay hints and ignores caller
supplied speculative hints and inline catalog dependencies. Required requests
and probes still use the catalog and the ordinary verified fetch path. A
partial policy binding has the same unavailable behavior. A policy error is
reported rather than replaced with a second implementation.

The resolver validates required job limits before optional selection. It passes
the smaller of its configured limits and the shared speculative limits to Rust;
Rust then charges optional files and bytes separately from required demand.
Inline catalog dependencies are optional candidates, so their size cannot
reject a required payload. Optional responses admitted to the VFS carry the
`speculative` flag for telemetry, including inline dependencies.

Tests may inject an explicit policy object to exercise transport behavior.
Such fakes are test dependencies, not production policy fallbacks. The policy
version in a persisted lookup identity records whether Rust prediction was
available, so observations from an unbound run cannot silently become
speculation in a later bound run. Metrics for an unbound run report zero
predictive candidates and speculative bytes; demand bytes and readiness still
reflect admitted required resources. A compile result does not depend on
speculation. Its absence can increase network waits and alter transport
telemetry.

The packaged browser gate runs generated packed shards with real WASM catalog
and prefetch bindings, including corrupt and mispartitioned shard rejection,
then exercises a worker resource round trip. Node resolver tests use fake
catalog and policy bindings for transport failures and cache behavior. Neither
test lane provides a JavaScript TeX scanner or packed shard decoder.
