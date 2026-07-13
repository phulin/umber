# Authored JavaScript Guidance

Use dependency-free browser-standard ES modules. Every browser API must be
injectable or guarded so the fast acceptance tests run under Node. Keep the
network resolver separate from compile retry and worker-controller policy.

Run authored tests with `node --test crates/umber-wasm/js/*.test.js`.

## File map

- `manifest-resolver.js`: validated immutable-manifest HTTP resolver.
- `manifest-resolver.d.ts`: public resolver declarations.
- `manifest-resolver.test.js`: resolver integrity, concurrency, hint, and cache tests.
