# Authored JavaScript Guidance

Use dependency-free browser-standard ES modules. Every browser API must be
injectable or guarded so the fast acceptance tests run under Node. Keep the
network resolver separate from compile retry and worker-controller policy.

Run authored tests with `node --test crates/umber-wasm/js/*.test.js`.

## File map

- `compile.js` / `compile.d.ts`: typed file/font `ResourceResolver` facade over `advance`/`provideResources` and public types.
- `compile.test.js`: file/font batching, hints, retry, progress, conflict, abort, and JavaScript-side limit tests.
- `persistent-cache.js` / `persistent-cache.d.ts`: distribution-scoped IndexedDB and in-memory verified-object stores.
- `persistent-cache.test.js`: key isolation and cache lifecycle tests.
- `worker-controller.js` / `worker-controller.d.ts`: main-realm timeout/abort controller.
- `worker-controller.test.js`: transfer fidelity and teardown tests.
- `worker-entry.js`: dedicated module-worker compile entry and transfer response.
- `html-preview.js` / `html-preview.d.ts`: CSP-validated scriptless iframe installer for canonical generated HTML.
- `source-map.js` / `source-map.d.ts`: DOM point to revision-checked rendered-source query helper.
- `manifest-resolver.js`: validated immutable-manifest HTTP resolver.
- `manifest-schema.js`: immutable manifest, lookup-key, path, and compatibility validation.
- `manifest-schema.test.js`: shared Rust/JavaScript manifest-selection fixture parity.
- `manifest-resolver.d.ts`: public resolver declarations.
- `manifest-resolver.test.js`: resolver integrity, concurrency, hint, and cache tests.
- `resource-resolver.js` / `resource-resolver.d.ts`: ordered typed provider
  composition with provider-scoped misses, final absence, and cancellation.
- `resource-resolver.test.js`: precedence, exact-key, failure, cancellation, and
  malformed-provider coverage for the composite facade.
