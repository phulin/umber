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

- `src/lib.rs`: exported persistent `CompilerSession`, low-level `advance`/`provideResources`/`applyPatch` boundary, revision metrics, and TypeScript surface.
- `src/catalog_boundary.rs`: persistent synchronous `CatalogSession` exports
  for canonical root validation, packed shard-byte admission and retention,
  request partitioning, exact lookup, and ordered selection plans owned by
  `umber-distribution`.
- `src/options.rs`: schema-1 DTO conversion into private session options, patches, and shared-VFS resource responses.
- `src/result.rs`: private engine result conversion into schema-1 attempt, output, diagnostic, and resource DTOs.
- `src/wire.rs`: schema-versioned host-neutral DTO authority, safe JavaScript
  integers, binary serializer, and derived TypeScript shapes; its generated
  `src/wire_schema.d.ts` custom section is checked against the Rust authority.
- `examples/wire_types.rs`: deterministic schema declaration generator used
  whenever schema-1 DTOs change.
- `src/result/`: focused binary-safe resource-request, metrics, and incremental-render conversion modules.
- `tests/it.rs`: wasm-bindgen boundary, lifecycle, and native fixed-point
  representation/failure parity tests over shared stabilization fixtures.
- `tests/virtual_font_acquisition.rs`: focused host-neutral WASM coverage for
  typed virtual-font resource retries and their recursive PDF resource closure.
- `assets/plain-source.lock`: exact TeX Live 2025 Plain, hyphenation, and TFM source identities.
- `assets/plain.fmt`: retained schema-11 regeneration input; it is not shipped.
- `assets/plain-format.json`: explicit schema-12 republication status metadata.
- `assets/cmu-serif-500-roman.woff2` / `assets/CMU-OFL.txt`: repository-only
  conformance fixtures and license; the npm runtime inventory excludes both.
- `js/`: authored ES modules and Node acceptance tests, including the unified resource facade and optional application-manifest file/font resolver.
- `browser-tests/`: dependency-free local HTTP and headless-Chrome package integration fixture.
- `browser-tests/node-project.mjs`: packaged Node TeX-bibliography-TeX acceptance test.
- `browser-tests/html-prototype.html`: two-engine line-baseline, shaping, negative-coordinate, rule, and fixed OpenType-math SVG projection prototype.
- `examples/`: minimal Plain and LaTeX-DVI module-worker browser examples
  shipped in the package.
- `package.json`: authored npm exports, package inventory, and distribution policy.
- `THIRD_PARTY_NOTICES.md`: Plain-format source provenance and redistribution notices.
