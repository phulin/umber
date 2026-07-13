# Umber WASM Guidance

This crate is the thin WebAssembly boundary around the host-neutral session in
`umber`. Rust bindings must not duplicate engine or retry policy. Authored
JavaScript owns asynchronous networking, persistent browser caching, worker
containment, and package ergonomics; it must remain usable with injectable
platform dependencies in Node tests.

Do not add `web-sys` merely to fetch. Large byte payloads cross as
`Uint8Array`, not JSON arrays or base64. Never derive distribution URLs from
untrusted TeX lookup names: resolve a validated manifest key, then fetch only
the manifest's validated content-addressed object name.

## Directory map

- `src/lib.rs`: exported `CompilerSession` and TypeScript surface.
- `src/options.rs`: strict conversion of JavaScript options and request keys.
- `src/result.rs`: binary-safe conversion of native attempts to discriminated JS results.
- `tests/it.rs`: wasm-bindgen boundary and lifecycle tests.
- `js/`: authored ES modules and Node acceptance tests.
