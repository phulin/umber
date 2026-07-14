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
- `cm-fonts.js` / `cm-fonts.d.ts`: packaged CM Unicode face loader and explicit OT1 text mapping helper.
- `manifest-resolver.js`: validated immutable-manifest HTTP resolver.
- `manifest-schema.js`: immutable manifest, lookup-key, path, and compatibility validation.
- `manifest-resolver.d.ts`: public resolver declarations.
- `manifest-resolver.test.js`: resolver integrity, concurrency, hint, and cache tests.
