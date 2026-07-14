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

- `src/lib.rs`: exported `CompilerSession`, low-level `advance`/`provideResources` boundary, and TypeScript surface.
- `src/options.rs`: strict conversion of JavaScript options, typed resource responses, and request keys.
- `src/result.rs`: binary-safe conversion of native typed resource batches and completed attempts to discriminated JS results.
- `tests/it.rs`: wasm-bindgen boundary and lifecycle tests.
- `assets/plain-source.lock`: exact TeX Live 2025 Plain, hyphenation, and TFM source identities.
- `assets/plain.fmt`: reproducible Umber-native Plain format image.
- `assets/plain-format.json`: published digest and compatibility metadata.
- `assets/cmu-serif-500-roman.woff2` / `assets/CMU-OFL.txt`: pinned browser-shapeable Computer Modern Unicode face and embedding license.
- `js/`: authored ES modules and Node acceptance tests, including the unified resource facade and optional application-manifest file/font resolver.
- `browser-tests/`: dependency-free local HTTP and headless-Chrome package integration fixture.
- `browser-tests/html-prototype.html`: two-engine line-baseline, shaping, negative-coordinate, and rule projection prototype.
- `examples/`: minimal module-worker browser example shipped in the package.
- `package.json`: authored npm exports, package inventory, and distribution policy.
- `THIRD_PARTY_NOTICES.md`: Plain-format source provenance and redistribution notices.
