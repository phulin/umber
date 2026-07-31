# Loaded Format Fixture Substrate

Status: implemented raw TeX82 and raw e-TeX 2.6 recipes

## Scope

`FormatRecipe`, `FormatFixture`, and `ensure_format` are the native fixture
boundary for generated Umber formats. A recipe describes every input that can
change the image or the meaning of a loaded run. A fixture is a validated image
plus that recipe identity. Construction and loaded execution are separate
operations: construction alone may execute a source containing `\dump`, and
the loaded runner cannot invoke format dumping.

The public raw TeX82 and raw e-TeX 2.6 recipes each have a construction source
containing only `\dump`; neither loads Plain TeX nor installs Plain macros. The
e-TeX recipe selects the extended canonical INITEX profile, so construction
installs the exact TeX82 plus e-TeX primitive registry and no pdfTeX layer.
Later raw pdfTeX recipes and package formats such as LaTeX extend the same data
model rather than adding profile-specific cache branches.

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
The reserved production sentinel is worker-owned: its exact one-argument form
enters the worker, while any trailing argument returns a worker error before
ordinary application dispatch. Unrelated arguments remain application-owned.
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
schema-validated deterministic image and `tex-observe` finalizes detached
command-v1 semantic plus geometry-v2 evidence. Evidence codec schema 1 uses
independently zero-based streams and hard limits of 1,000,000 events per
stream, 1 MiB per canonical event, 256 KiB per encoded string, nesting depth
64, and 64 MiB total. Decoding rejects impossible frame counts before event
allocation, unknown fields, noncanonical encoding, gaps or duplicates,
stream-kind confusion, trailing bytes, malformed values, and every limit
violation before deserializing the affected payload.

Worker protocol 2 authenticates the complete success or error envelope,
recipe identity, image digest, and evidence digest under the per-child key.
The evidence-aware format identity is producer contract 2 and includes the
evidence schema and limits, so image-only entries are disjoint.
`FormatCacheStore` compound entry schema 2 writes the validated image and
opaque caller-validated evidence to one entry in one publication. The legacy
image-only API remains a separate entry schema for external cache CLI users;
the evidence-aware fixture path never accepts it. `FormatCacheStore` writes an entry to a
same-directory temporary file, syncs it, publishes with a no-clobber rename,
and syncs the containing directory. Per-key exclusion remains held through
validation, quarantine, construction, and publication, making invalid-entry
replacement identity-safe and ensuring concurrent callers construct a missing
or semantic-invalid key only once. Existing authority components and entries
are inspected without following links. Racing
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

Raw e-TeX reload uses the generic `EngineMode::ETex` registry reconstruction:
TeX82 and e-TeX expandable and unexpandable meanings are registered as live
implementations, while pdfTeX-only meanings remain absent. A missing or wrong
profile, primitive registry, semantic schema, producer contract, source,
resource closure, fixed clock, guard set, or build configuration selects a
different cache identity or fails validation; loading never guesses a legacy
profile.

`FormatRecipe::production_pdftex14027` is the explicit public production
pdfTeX 1.40.27 recipe. Its format name is `production`, matching the pinned
oracle's `-fmt=production` identity. It selects the exact `Pdftex14027` profile and
fingerprint, prepares the combined TeX82, e-TeX 2.6, and pdfTeX primitive
registry and parameter defaults in INITEX, and reaches construction completion
only through its own `\dump`. It uses the same authenticated worker,
content-addressed cache, validation, and live-registry reconstruction as the
raw recipes. Its engine mode, prepared-registry hash, source and resource
closure, schemas, producer/build contract, clock, and positive guard set make
its identity disjoint from raw TeX82 and raw e-TeX.

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

Format construction and loaded execution have separate parity contracts. A
recipe-owned construction that completes through `\dump` compares exact
canonical command events and geometry and validates publication and reload, but
does not compare terminal or log bytes. Dump-time allocator, string-pool, and
serialization reports are not portable output. This is a phase-level rule, not
a fixture-name special case or diagnostic grammar. Loaded jobs and ordinary
non-dump jobs continue exact meaningful terminal and log comparison, and
construction effects never become loaded-job comparison input.

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
properties, raw TeX82 and raw e-TeX cache reuse and live-registry reload, the ordinary
`main-control/hyphenation-data` corpus case through raw-TeX82 loaded execution,
declared loaded-job `\input` and TFM replay with exact output-channel
assertions, and one explicit fresh-versus-loaded semantic-state invariant per
raw profile. The e-TeX invariant uses extension-owned integer state and
canonical observations, so it proves that restored live meanings operate on
the same immutable base without comparing construction terminal or log. TeX82
§§1250–1252 make that first migrated case a useful loaded-state witness:
dumping finalizes the trie, so its too-late `\patterns` scan publishes 77
canonical events rather than the synthetic fresh-universe path's 78.
The production pdfTeX invariant reuses one cache identity, proves that loaded
TeX82, e-TeX, and pdfTeX live meanings include `\pdfsavepos`, and verifies that
the enabled PDF backend and default `\pdfoutput=0` survive the immutable image
without a frozen first-shipout mode, while the construction world, effects,
artifacts, provenance, interaction, and clock do not cross into the loaded
runtime.
Every engine execution has positive finite fuel, and all actual test runs use
the repository timeout/RSS guard.

The blessed raw-TeX82 loaded oracle profile additionally owns an exact
172-case allowlist. It preserves the independently reviewed original 35-case
cohort and 55 ordinary main-control jobs, excluding the three separately owned
hyphenation/error/final-cleanup fixtures, and adds all 18 alignment, 34 math,
and 30 page-output jobs. One
cached raw format identity serves all selected jobs; inputs, TFMs, terminal
lines, and interaction mode remain job-local after restore. The alignment
cohort pins two job-local TFMs, 17 clean statuses plus exact
`fatal:confusion(256 spans)`, and ten empty plus eight file DVI channels.
The math cohort pins 17 job-local TFMs, one terminal-interaction job, 34 clean
statuses, and 12 empty plus 22 file DVI channels.
The page-output cohort pins 13 job-local TFMs, 30 clean statuses, three empty
plus 26 file DVI channels, and the existing pinned DVI xfail for
`special-in-shipped-hbox`.
Worker-boundary regressions additionally replace stale and wrong sibling
candidates before selection, replace a pathname after its inode is anchored,
exercise the current-image worker entry, and submit a decoder-valid image with
a forged authenticator. Every attestation or authentication failure is checked
before publication and leaves the recipe key absent from the cache.
