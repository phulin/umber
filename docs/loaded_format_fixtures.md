# Loaded Format Fixture Substrate

Status: universal persistent provider migration complete

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

## Universal prepared-format test provider

Every test that drives the complete Umber command-to-output pipeline must use
one provider contract. The provider accepts a complete `FormatRecipe`, obtains
its authenticated compound cache entry with `ensure_format`, loads the returned
`FormatFixture` into a provider-created fresh `World`, and runs one authored
root plus ordered typed `LoadedFormatResource` values through
`LoadedFormatFixture::run`. It is not a family adapter: command minifixtures,
TRIP, e-TRIP, Story, and Gentle supply data to the same API and may not own
private cache, INITEX, image-decoding, dump, or captured-runner branches.

The provider belongs in the native `umber` fixture boundary beside
`FormatRecipe` and `FormatFixture`. Tools and integration tests may supply the
registered worker launcher required by their executable, but launcher routing
does not change recipe identity or provider behavior. The operation has two
explicit phases:

1. `prepare(recipe)` resolves the persistent native store and returns an
   authenticated `FormatFixture` with detached construction evidence. This is
   the only operation allowed to construct or dump a format.
2. `run(fixture, job)` creates a fresh memory `World` from the request's job
   clock, registers only that request's terminal input, calls
   `FormatFixture::load`, applies the remaining typed job configuration, and calls
   `LoadedFormatFixture::run`. It cannot construct, dump, or reuse a loaded
   `Universe` or accept a caller-created `World` whose mutable state cannot be
   proven fresh.

The implemented boundary is `PreparedFormatProvider`. Its production
`from_environment` constructor resolves the platform store, while `with_store`
accepts an already-resolved store for hermetic tests without changing provider
behavior or adding a fallback. `prepare` accepts `FormatRecipe`; `run` accepts
`PreparedFormatJob`, whose job clock, profile, output backend, interaction,
error-context widths, positive guards, provenance demand, authored-root
name/kind/bytes, typed resources, terminal lines, and command observer are all
explicit. The request has no `World` field.

The provider's loaded resource host gives job resources precedence, then may
reopen authenticated recipe resources. This preserves the ordinary format
boundary where a job can request an input or TFM that was also present during
construction without making the job duplicate format-owned bytes or weakening
the recipe identity. In particular, redefining a preloaded font by its original
name remains deterministic in a fresh `World`.

`prepare` uses `FormatCacheStore::from_environment`, hence the existing native
Umber platform cache (`$XDG_CACHE_HOME/umber` when set, otherwise the platform
cache directory, such as `$HOME/.cache/umber` on Linux). This is ignored,
generated runtime data, not a repository `target/` artifact, a provisioned
native test asset, or a committed fixture. Primary checkouts, linked worktrees,
and repeated local processes therefore share complete identities. CI may set
`XDG_CACHE_HOME` to a writable job cache restored by its ordinary cache
mechanism; a cold or deliberately unshared CI job constructs locally. No
network, setup script, corpus copy, or committed generated binary is added.
An unavailable or unwritable platform cache is an explicit provider error,
not permission to fall back to an invocation-local directory or source-run the
job.

The existing recipe identity remains the sole reuse authority. It covers the
container/ABI/lookup and observation schemas, producer and build contracts,
engine profile and live registry, format name, ordered construction source and
typed Input/TFM closure by logical identity and bytes, distribution identity,
fixed construction clock, interaction and error-context widths, and finite
fuel/wall/RSS guards. A family label, checkout path, cache root, job source,
job resources, or process lifetime is not part of that identity. Story and
Gentle share one Plain recipe because their complete construction closure is
identical; TRIP and e-TRIP retain distinct recipe identities because their
engine profiles, construction sources, names, and closures differ. Cache
schema, producer, source, resource, profile, build, evidence, or guard changes
select a new key rather than upgrading an old entry.

All existing `FormatCacheStore` security and recovery rules apply unchanged:
anchored no-follow authority, a persistent per-key interprocess lock held
through validation/quarantine/construction/publication, authenticated bounded
worker protocol, complete image and evidence validation, atomic no-clobber
publication, and removal of only operation-owned temporary/quarantine names.
Corruption is quarantined and regenerated while holding the key lock. Ordinary
cleanup never deletes valid persistent entries; users and CI may evict the
whole platform cache as a performance-only action when no provider process is
using it. A cache hit is fully offline. A miss is also offline when the recipe
contains its complete construction source and resource bytes; missing external
input must fail before preparation, never trigger acquisition.

Construction evidence and loaded-job observations are separate typed channels.
`FormatFixture::construction_evidence` is identical on miss and hit and may be
compared only as INITEX/dump evidence; construction terminal, log, effects, and
status never enter loaded-job acceptance. Each loaded run owns a new `World`,
`Universe`, clock episode, root source, resource host, observer, checkpoints,
effects, and output assembly. Its job request explicitly supplies engine
profile compatibility, job clock, interaction mode, TeX82 error-context
widths, its own finite command/wall/RSS limits, backend/output policy,
provenance demand, authored-root identity, source kind and bytes, ordered
Input/TFM resources, terminal input where needed, and observers. Construction
uses only the recipe's clock and guards, which remain cache-identity inputs; a
job request's clock and guards are runtime controls and never select or mutate
a format entry. The provider rejects profile mismatch, unsupported
backend/profile combinations, or an unbounded construction or job guard.
Geometry remains captured and reported but advisory; command-v1, terminal,
log, effects, status, and normalized DVI remain governed by each fixture's
existing acceptance contract.

The job's provenance demand is applied only after authenticated format decode,
before execution begins. Diagnostics-only batch jobs therefore retain no
rendered-source artifact sidecar, while parity and telemetry jobs which inspect
post-shipout provenance explicitly select the rendered-source consumer. This
operational choice is excluded from recipe identity and format bytes, and every
fresh job selects it independently.

### Full-pipeline call-site inventory and target state

| family                 | current full-pipeline helpers and callers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | current preparation                                                                                                                                                       | required target                                                                                                                                                                                                                                                                      |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| command minifixtures   | `tools/tex-command-stream/src/semantic.rs`: `loaded_format_recipe` -> `execute_raw_tex82_loaded`, `execute_raw_etex26_loaded`, `execute_production_pdftex14029_loaded` -> `execute_loaded_format`; persistent-reuse and fresh-job controls in `tools/tex-command-stream/tests/it/command_semantic.rs`                                                                                                                                                                                                                                                                                         | migrated: each complete recipe prepares through `PreparedFormatProvider`; each caller supplies a complete `PreparedFormatJob`, and the provider creates the fresh `World` | complete: production resolves the persistent environment store; hermetic controls inject one scoped store and prove stable/disjoint identities, authenticated reuse across independent provider instances, and mutable-state isolation                                               |
| TRIP                   | `crates/umber/tests/it/e2e_conformance.rs`: ignored `e2e_conformance_trip_canonical` -> `run_two_phase_fixture` via `trip_format_recipe`                                                                                                                                                                                                                                                                                                                                                                                                                                                      | migrated: the complete TeX82 recipe prepares through the persistent provider, which owns each fresh loaded job                                                            | complete: construction evidence and advisory geometry remain separate from loaded command/output channels and normalized-DVI acceptance; hermetic controls prove authenticated warm reuse and fresh mutable state                                                                    |
| e-TRIP                 | same file: ignored `e2e_conformance_etrip` -> `run_two_phase_fixture` via `trip_format_recipe`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | migrated through the same provider helper with a distinct complete e-TeX recipe identity                                                                                  | complete: the typed profile, construction closure, loaded resources, runtime controls, observation channels, and acceptance policy remain explicit                                                                                                                                   |
| legacy Story/Gentle    | same file: `e2e_conformance_story` and `e2e_conformance_gentle` -> `run_plain_fixture_case` -> `run_file_with_plain_format`                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | migrated: the shared complete Plain recipe prepares through `PreparedFormatProvider`; each document executes as a fresh loaded job                                        | complete: the staged wrapper's `plain.tex` prefix is construction-only; document and non-preload resources are typed job inputs, and provenance plus the existing normalized-DVI policy remain preserved                                                                             |
| canonical Story/Gentle | same file: `e2e_conformance_story_canonical` and `e2e_conformance_gentle_canonical` -> `run_plain_fixture_case_canonical`; `canonical_ligature_group_boundaries_match_reference_dvi`, `canonical_rule_space_factor_reset_matches_reference_dvi`, `canonical_alignment_leading_tabskip_matches_reference_dvi`, `canonical_rule_follows_pending_characters_in_reference_dvi`, `canonical_relax_breaks_ligatures_in_reference_dvi`, `canonical_display_equation_number_preserves_formula_dvi`, and `canonical_math_group_singleton_ord_matches_reference_dvi` -> `run_file_in_process_canonical` | migrated: Story/Gentle use the persistent Plain recipe, while the seven self-contained fixtures use the persistent raw-TeX82 recipe through the same provider             | complete: each route supplies an explicit clock, profile/backend, Nonstop interaction, error widths, finite guards, terminal input, observer, authored root, and typed resources; recipe identity preserves the source INITEX allocation namespace without an output remapping layer |

The Gentle profiling binary is a performance/session tool rather than one of
these parity test families; this migration must not silently broaden into its
incremental-session architecture. Format-construction unit tests remain direct
tests of `ensure_format`, worker protocol, and cache storage because they test
the preparation operation itself rather than a full loaded pipeline.

The Plain construction closure is exact rather than host-discovered: it owns
the pinned `plain.tex` and `hyphen.tex` bytes plus every TFM named by
`parity_harness::PLAIN_PRELOAD_FONTS`. Those construction resources are loaded
from repository-controlled inputs before `prepare`; absence fails locally and
cannot fall through to a system TeX tree or network lookup. Document roots and
fonts not in that preload inventory remain typed job resources.

### Ordered implementation decomposition

Implementation proceeds linearly so no two changes own the same helper:

1. Add the provider and behavioral/security controls, including persistent
   root resolution, cold miss/warm hit across independent provider instances,
   concurrent exactly-once construction, corrupt-entry recovery, offline
   closure, profile mismatch, finite guards, and two fresh-job state checks.
2. Command minifixtures migrated; their process-local cache/counters are removed.
3. Migrate the shared TRIP/e-TRIP helper and remove its invocation-local cache
   and duplicate preparation.
4. Add the complete Plain recipe/resource split and migrate legacy and
   canonical Story/Gentle to it. Keep self-contained committed-DVI callers on
   the generic persistent raw-TeX82 recipe so their standalone INITEX state,
   including font allocation order, is unchanged.
5. The final audit removed the staged-World resource host, captured direct
   INITEX/dump/load runner, checkpoint adapter, manual failure adapter, and
   their imports. Definition-anchored controls enumerate every caller above
   and reject private cache, worker, decode, and construction ownership in
   family helpers. Direct construction/security/unit coverage remains at the
   generic format boundary.

## Identity

The cache key is a canonical, domain-separated encoding of:

- format-container schema, ABI, and lookup-configuration fingerprints;
- command-state and command-observation schema versions;
- the installed INITEX universe's actual primitive registry and live meanings;
- the producer implementation version and compiled feature contract;
- ordered construction sources and typed resources, including logical names,
  kinds, byte lengths, and SHA-256 hashes;
- distribution identity and fixed TeX job clock;
- construction interaction mode and TeX82 error-context widths;
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
`EngineSession`. The ordered typed-resource closure and finite
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

Before releasing the authenticated construction request, the parent samples
the worker's baseline RSS and rejects an already-exceeded bound. It continues
sampling worker wall time and RSS independently of command return or
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
The evidence-aware format identity is producer contract 5 and includes the
evidence schema and limits, so image-only entries are disjoint.
`FormatCacheStore` compound entry schema 2 writes the validated image and
opaque caller-validated evidence to one entry in one publication. The legacy
image-only API remains a separate entry schema for external cache CLI users;
the cache identity pins which entry kind its API may access, so legacy load or
store calls cannot inspect, quarantine, or publish an evidence-aware key.
`FormatCacheStore` writes an entry to a same-directory temporary file, syncs
it, publishes with a no-clobber rename, and syncs the containing directory.
Per-key exclusion remains held through
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

A `FormatFixture` loads by consuming its validated bytes into a fresh `World`
and `Universe`, then reinstalling the selected profile's live primitive
implementations. Admission releases the encoded buffer before construction and
drains or moves the decoded rows into their destination owners; it does not
retain a parallel format image. The returned `LoadedFormatFixture` owns that
fresh universe until it constructs one retained `EngineSession` for a job.

DVI font numbers are the immutable internal font allocation index minus one,
as specified by tex.web §§617 and 642. The frozen font rows preserve that
allocation order across format loading, and tex.web §1257's lookup of an
equivalent loaded font retains its existing number. A loaded job therefore
does not own a second numbering registry: definitions, page artifacts, and
postamble definitions all expose the same format-persistent font identity.

Raw e-TeX reload uses the generic `EngineMode::ETex` registry reconstruction:
TeX82 and e-TeX expandable and unexpandable meanings are registered as live
implementations, while pdfTeX-only meanings remain absent. A missing or wrong
profile, primitive registry, semantic schema, producer contract, source,
resource closure, fixed clock, guard set, or build configuration selects a
different cache identity or fails validation; loading never guesses a legacy
profile.

`FormatRecipe::production_pdftex14029` is the explicit public production
pdfTeX 1.40.29 recipe. Its format name is `production`, matching the pinned
oracle's `-fmt=production` identity. It selects the exact `Pdftex14029` profile and
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
canonical command events, retains advisory geometry comparison, and validates
publication and reload, but does not compare terminal or log bytes. Geometry
differences are reported and counted without affecting acceptance. Dump-time allocator, string-pool, and
serialization reports are not portable output. This is a phase-level rule, not
a fixture-name special case or diagnostic grammar. Loaded jobs and ordinary
non-dump jobs continue exact meaningful terminal and log comparison, and
construction effects never become loaded-job comparison input.

Loaded jobs use TeX82 §§61, 534, 536, 537, 642, and 1333 framing through the
generic retained session. A named retained root lets the command input stack
own its balanced file-opening events; an unnamed compatibility root retains
the driver's explicit opening. Completion serializes any prepared DVI pages
before printing §642's exact byte count, while zero-page jobs supply no DVI
descriptor. Host-side transcript evidence joins the memory World's already
committed terminal/log prefix to the live effect suffix: §638 shipout may have
drained startup and diagnostic writes before §1333 appends the job tail, but
the two storage locations remain one ordered observable channel.

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
176-case allowlist. It preserves the independently reviewed original 35-case
cohort and 55 ordinary main-control jobs, excluding the three separately owned
hyphenation/error/final-cleanup fixtures, and adds all 18 alignment, 34 math,
33 page-output jobs, and the bounded line-breaking paragraph-shape job. One
cached raw format identity serves all selected jobs; inputs, TFMs, terminal
lines, and interaction mode remain job-local after restore. The alignment
cohort pins two job-local TFMs, 17 clean statuses plus exact
`fatal:confusion(256 spans)`, and ten empty plus eight file DVI channels.
The math cohort pins 17 job-local TFMs, one terminal-interaction job, 34 clean
statuses, and 12 empty plus 22 file DVI channels.
The page-output cohort pins 13 job-local TFMs, 33 clean statuses, four empty
plus 29 file DVI channels, and no DVI xfails.
Worker-boundary regressions additionally replace stale and wrong sibling
candidates before selection, replace a pathname after its inode is anchored,
exercise the current-image worker entry, and submit a decoder-valid image with
a forged authenticator. Every attestation or authentication failure is checked
before publication and leaves the recipe key absent from the cache.
