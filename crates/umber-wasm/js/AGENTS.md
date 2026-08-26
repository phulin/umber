# Authored JavaScript Guidance

Use dependency-free browser-standard ES modules. Every browser API must be
injectable or guarded so the fast acceptance tests run under Node. Keep the
network resolver separate from compile retry and worker-controller policy.

Run authored tests with `node --test crates/umber-wasm/js/*.test.js`.

## File map

- `compile.js` / `compile.d.ts`: typed file/font `ResourceResolver` facade over `advance`/`provideResources` and public types.
- `session-driver.js`: shared direct and worker-realm session retry,
  cancellation, progress, and disposal core.
- `compile.test.js`: file/font batching, hints, retry, progress, conflict, abort, and JavaScript-side limit tests.
- `persistent-cache.js` / `persistent-cache.d.ts`: distribution-scoped IndexedDB and in-memory verified-object stores.
- `persistent-cache.test.js`: key isolation and cache lifecycle tests.
- `worker-controller.js` / `worker-controller.d.ts`: main-realm timeout/abort controller.
- `worker-rpc.js`: shared one-shot and retained-worker request correlation,
  timeout, progress, and teardown core.
- `worker-controller.test.js`: transfer fidelity and teardown tests.
- `worker-entry.js`: dedicated module-worker compile entry and transfer response.
- `html-preview.js` / `html-preview.d.ts`: CSP-validated scriptless iframe installer for canonical generated HTML.
- `html-patch.js` / `html-patch.d.ts`: public typed snapshot/patch DOM mount facade.
- `html-patch-dom.js`: safe DOM construction, mutation, and user-state preservation.
- `html-patch-model.js`: snapshot/patch validation and canonical model simulation.
- `html-patch-resources.js`: content-addressed resource staging and lifetime.
- `html-patch-shared.js`: internal schema constants, bounded values, and adapter utilities.
- `source-map.js` / `source-map.d.ts`: DOM point to revision-checked rendered-source query helper.
- `manifest-resolver.js`: immutable-manifest HTTP/cache adapter over the
  required typed `umber-wasm` catalogue plans; it owns transport policy but no
  catalogue schema, packed-byte decoder, JSON shard bridge, or selection
  policy. It transports and caches shard `Uint8Array` bytes and supplies each
  touched shard once to the retained Rust catalogue session.
- `manifest-schema.js`: request/response wire identity adapters only; catalogue
  parsing, duplicate rejection, authentication, partitioning, serialization,
  format lookup, and batch selection remain in `umber-distribution` behind the
  WebAssembly boundary.
- `manifest-resolver.d.ts`: public resolver declarations.
- `manifest-resolver.test.js`: resolver integrity, concurrency, hint, and cache tests.
- `resource-resolver.js` / `resource-resolver.d.ts`: ordered typed provider
  composition with provider-scoped misses, final absence, and cancellation.
- `resource-resolver.test.js`: precedence, exact-key, failure, cancellation, and
  malformed-provider coverage for the composite facade.
