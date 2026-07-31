# Loaded Format Fixture Substrate

Status: implemented first slice for raw TeX82 command-semantic fixtures

## Scope

`FormatRecipe`, `FormatFixture`, and `ensure_format` are the native fixture
boundary for generated Umber formats. A recipe describes every input that can
change the image or the meaning of a loaded run. A fixture is a validated image
plus that recipe identity. Construction and loaded execution are separate
operations: construction alone may execute a source containing `\dump`, and
the loaded runner cannot invoke format dumping.

The first recipe is raw TeX82. Its construction source contains only `\dump`;
it does not load Plain TeX or install Plain macros. Later raw e-TeX and pdfTeX
recipes, and package formats such as LaTeX, extend the same data model rather
than adding profile-specific cache branches.

## Identity

The cache key is a canonical, domain-separated encoding of:

- format-container schema, ABI, and lookup-configuration fingerprints;
- command-state and command-observation schema versions;
- the installed INITEX universe's actual primitive registry and live meanings;
- the producer implementation version and compiled feature contract;
- ordered construction sources and typed resources, including logical names,
  kinds, byte lengths, and SHA-256 hashes;
- distribution identity and fixed TeX job clock;
- cumulative command fuel, wall-time, and resident-set limits; and
- producer-derived build configuration.

The encoding contains no Cargo target directory, executable path, checkout
path, temporary path, or host cache path. Changing any semantic or generation
guard input selects a disjoint key.

## Construction

`ensure_format` requires an explicit `FormatWorkerLauncher` from its binary
consumer, then asks `FormatCacheStore` for a validated entry. On a miss, it
sends the complete recipe to a dedicated native worker process. The request
contains the expected cache identity; the worker reconstructs the recipe and
recomputes that identity before creating a memory `World`, installing the
selected fresh primitive profile, and driving the retained
`CanonicalEngineSession`. The ordered typed-resource closure and finite
command-fuel limit remain inside that worker. A production consumer calls
`dispatch_format_worker` before ordinary argument parsing and passes
`FormatWorkerLauncher::production`; a libtest consumer installs
`register_format_worker_test_bootstrap!` and passes its exact registered route.
No executable path, arbitrary argument, or environment value is used to infer
support. An absent capability fails before current-image selection, spawn, or
cache publication. Production opens `/proc/self/exe` and executes that stable
descriptor through `/proc/self/fd`; a test image re-executes itself with its
exact filter, so the bootstrap consumes worker mode once before ordinary test
concurrency can begin. The current process image is the trust anchor: no sibling
path, public version, feature string, build ID, or hash supplied by selected
code participates in authentication. Consequently a stale, wrong, or
attacker-replaced sibling is never selected, while replacement of the proc
pathname after opening cannot change the executed inode. The parent creates a
fresh random authentication key for that child episode and prefixes it to the
private stdin stream. A fixed prefix and fixed-width length frame the
independently serialized request; the worker rejects truncated, oversized, or
host-unrepresentable lengths before reserving payload storage. The response
uses the same bounded framing inside the independently bounded stdout stream.
Parent and child keep every key copy and HMAC pad in explicitly zeroizing
storage; their owned stdin/stdout handles close on every return, including
spawn, protocol, authentication, and construction failures. The worker
authenticates the complete result envelope with HMAC-SHA-256, binding the
protocol, recipe identity, image digest, and success or diagnostic payload to
the trusted child.

The parent samples worker wall time and RSS independently of command return or
cooperative engine checkpoints while bounded readers concurrently drain both
worker pipes. Standard output is limited to the 256 MiB format-image limit plus
64 KiB of protocol framing, and diagnostic standard error is limited to 1 MiB.
The parent kills and reaps the worker when either resource bound or either
pipe limit is exceeded, so one non-returning, allocating, or pipe-saturating
command cannot hold the fixture harness. Reader, writer, and process errors
also terminate and reap before returning; bounded crash diagnostics remain
attached to the error. A crash, malformed response, or authentication failure
publishes nothing and a later call starts an independent worker. The response
repeats the recipe identity and image SHA-256; the parent verifies both,
verifies the child-episode authenticator, and performs a complete frozen-format
decode before cache publication.
Reader completion and wall-deadline classification share one synchronized
event state. At the deadline, the supervisor holds that state while sampling
the worker's Linux pidfd: readiness is reaped into the actual exit status
before classification, while non-readiness is the live-at-deadline
linearization point. A reader result published before that locked arbitration
participates in the decision, so an exited worker with both pipes closed
completes normally even when the preceding `try_wait` observed it live. An
exited worker with an unresolved inherited pipe, or a worker live at the pidfd
sample, reaches the deadline immediately; this finitely bounds both cases.
Reader publication wakes the supervisor, while a two-millisecond maximum wait
keeps live-child RSS sampling continuous.
Platforms without supported RSS supervision reject construction explicitly.
On Linux both the cooperative worker check and the parent supervisor convert
`/proc/*/statm` resident pages with the checked runtime page size; unavailable,
invalid, or overflowing measurements fail closed. If only the supervised
process's proc entry vanishes after it was observed live, the parent first
reobserves process exit and drains completed pipes. A confirmed exit continues
through ordinary authenticated completion or crash handling; a still-live
process, malformed accounting data, or any other accounting failure remains a
fail-closed `ResidentSetUnsupported` error.
These internal guards complement the outer `scripts/run-umber-guarded.py`
defense.

After a successful construction episode, the quiescent `Universe` produces a
schema-validated deterministic image. `FormatCacheStore` writes an entry to a
same-directory temporary file, syncs it, publishes with a no-clobber rename,
and syncs the containing directory. Per-key exclusion makes invalid-entry
quarantine identity-safe against peer replacement. Existing authority
components and entries are inspected without following links. Racing
publishers accept the already-valid winner. A partial temporary file is never
visible; a stale, truncated, mismatched, checksum-invalid, or decoder-invalid
destination is removed and regenerated.

Generated entries live only in the platform cache or an explicitly supplied
test cache. They are ignored runtime data and are never fixture authorities in
Git.

## Loading and execution

A `FormatFixture` loads by decoding its validated bytes into a fresh `World`
and `Universe`, then reinstalling the selected profile's live primitive
implementations. The returned `LoadedFormatFixture` owns that fresh universe
until it constructs one retained `CanonicalEngineSession` for a job.

Only immutable format state crosses the boundary. The format container excludes
the host `World`, open input and output state, interaction and runtime controls,
effect journal, provenance records, checkpoints, artifacts, and memoized or
state-hash caches. Loading therefore cannot inherit a construction host or
runtime episode. Live meanings are reconstructed from the profile registry
after decode rather than serialized function pointers or an ambient executor.

The loaded runner accepts a root source and typed resources, sets finite
cumulative command fuel, and returns structured semantic output. Its retained
result includes the job-local ordered mode transitions and exact fatal
terminal state; neither participates in immutable format identity. A TeX
fatal stop is a completed job outcome rather than a worker, guard, or runner
failure. It has no `dump_format` method and no dump flag. The compatibility
fresh runner remains an explicitly named test seam used only for the small
fresh-versus-loaded matrix; it is not an automatic fallback from a cache or
load failure.

The recipe's source and typed resource closure belong to format construction
and therefore participate in the format-cache identity. Inputs and TFMs
declared by an individual loaded job cross the separate
`LoadedFormatResource` boundary after restore. They preserve the host's
logical lookup key, resolved input name, source kind, and typed font
fulfillment, but do not alter the shared format-cache identity.

Loaded jobs use TeX82 §§61, 534, 536, 537, 642, and 1333 framing through the
generic retained session. A named retained root lets the command input stack
own its balanced file-opening events; an unnamed compatibility root retains
the driver's explicit opening. Completion serializes any prepared DVI pages
before printing §642's exact byte count, while zero-page jobs supply no DVI
descriptor.

## Verification

The substrate tests cover identity invalidation, byte-identical independent
builds, cache failure atomicity, concurrent publication, corrupt-entry
recovery, adversarial symlink authority, format schema and exclusion
properties, live-registry reload, the ordinary
`main-control/hyphenation-data` corpus case through raw-TeX82 loaded execution,
declared loaded-job `\input` and TFM replay with exact output-channel
assertions, and one explicit fresh-versus-loaded semantic-state invariant. TeX82
§§1250–1252 make that first migrated case a useful loaded-state witness:
dumping finalizes the trie, so its too-late `\patterns` scan publishes 77
canonical events rather than the synthetic fresh-universe path's 78.
Every engine execution has positive finite fuel, and all actual test runs use
the repository timeout/RSS guard.
Worker-boundary regressions additionally replace stale and wrong sibling
candidates before selection, replace a pathname after its inode is anchored,
exercise the current-image worker entry, and submit a decoder-valid image with
a forged authenticator. Every attestation or authentication failure is checked
before publication and leaves the recipe key absent from the cache.
