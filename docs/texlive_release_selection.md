# TeX Live Release Selection

Status: proposed implementation contract.

## Objective

Umber should select an annual TeX Live runtime distribution, initially from
2020 through 2026, without coupling package selection to the canonical TeX,
e-TeX, or pdfTeX implementation profile. A compile must resolve the requested
year to one immutable downloaded distribution and must report that resolved
identity. Native and browser frontends must select identical bytes for the
same year.

The design deliberately does not introduce a release-catalog manifest, a test
matrix manifest, a format lock, or a compatibility-attestation lock. Locks and
manifests are reserved for files acquired from outside the repository. Static
product policy belongs in typed source code, generated artifacts carry their
own existing container identity, and test cases belong in ordinary Rust or
script-owned test tables.

## Ownership rule for locks and manifests

A lock or manifest is appropriate only at an acquisition boundary:

- a lock may pin the length and cryptographic digest of a TeX Live archive,
  ISO slice, package database, or other downloaded upstream file;
- a published distribution manifest may name and authenticate objects that a
  client downloads; and
- the CLI may require the expected digest of a downloaded root manifest.

The following are not acquisition boundaries and must not gain new locks or
manifests:

- the list of TeX Live years supported by an Umber release;
- aliases, default selection, URLs, and CLI conflict rules;
- generated LaTeX and pdfLaTeX formats;
- the compatibility test matrix and its applicability rules;
- evidence that a current pdfTeX executable can stand in for an older one; and
- local test inputs or receipts produced by a test run.

When a downloaded distribution is already authenticated by its root manifest,
do not add another lock that restates the same object inventory. Derived local
files should refer to the resolved distribution identity in their normal
typed metadata or cache key rather than through a parallel lock file.

## Typed release policy

`umber-distribution` should define a small closed release identifier type and
the production frontend should own a compile-time table of release policy:

```rust
struct TexliveReleaseSpec {
    year: TexliveYear,
    distribution: &'static str,
    root_url: &'static str,
    root_ahash64: &'static str,
}
```

This is product configuration, not downloaded data, so it remains ordinary
reviewed Rust. Construction validates that years are unique, URLs use HTTPS,
distribution names are nonempty, and root digests use the canonical aHash64
encoding. A unit test asserts the intended contiguous supported range and the
single compiled-in default. The browser projection is generated from or calls
through the same Rust authority; it must not maintain a JSON shadow table.

The initial command-line surface is:

```text
umber run --texlive 2020 document.tex
umber run --texlive 2026 document.tex
```

`--texlive YEAR` conflicts with `--distribution` and
`--distribution-ahash64`. The latter pair remains the low-level escape hatch
for an arbitrary authenticated hosted root or local mirror. No flag selects
the compiled-in default release. `watch` accepts the same controls.

Avoid a remotely resolved `latest` alias. It would make identical invocations
select different bytes without an Umber update. If a user-facing `latest`
alias is added, it must mean the compiled-in default and print the concrete
resolved year, distribution name, URL, and digest before acquisition.

The release table contains a digest because the root manifest is downloaded;
this is precisely an acquisition pin. It does not make the table a second
manifest.

## Annual source acquisition and publication

The existing authenticated runtime-source path in
`scripts/texlive_release.py` should accept a year and select the corresponding
upstream acquisition lock. Per-year locks may be used here because their
records describe downloaded archives and ISO byte ranges. Provisioning should
accept:

```text
python3 scripts/provision.py runtime-source --texlive 2020
python3 scripts/provision.py snapshot --texlive 2020
```

Use the initial/DVD release for every supported year, matching the existing
2026-03-01 policy. Do not mix initial releases and end-of-cycle `tlnet-final`
trees under the same year selector. A later corrected snapshot needs a new
concrete distribution name; the product table may move the year to it in a
reviewed Umber release.

Publication runs the existing deterministic scanner and publisher once for
each annual tree. Every published root has a permanent year-specific URL and
unique distribution identity. Object storage remains content-addressed and
shared, so equal files across years need not be uploaded twice. Published
prefixes are immutable.

The package database, runtime archive, and generated map inputs are all
downloaded or extracted acquisition inputs and may therefore remain covered
by the existing acquisition locks and distribution manifest. Publication
configuration, inventory floors, supported years, and format names stay in
code and command-line arguments.

## Format and cache identity

LaTeX and pdfLaTeX formats are generated local artifacts, not downloaded
authorities. Do not create per-year format locks or format manifests. Build a
format from the selected distribution and store its resolved distribution
name and root-manifest digest in the existing frozen-format metadata and cache
identity.

A cached format is reusable only when all of these agree:

- engine and engine version;
- frozen-format schema;
- format name and construction profile;
- resolved distribution name and root-manifest digest; and
- exact construction-input closure already recorded by the format machinery.

The cache rejects a format built from 2026 while running against 2020, even if
both logical format names are `latex`. No additional lock file is involved.

## pdfTeX parity strategy

Distribution compatibility and engine conformance are separate test axes.
Multiplying every semantic fixture by every TeX Live year would conflate them
and create redundant authority.

### Canonical engine tier

Keep pdfTeX 1.40.29 as the sole pdfTeX semantic and byte-level oracle for
primitive behavior. Existing command-semantic, primitive PDF, TRIP, e-TRIP,
and focused DVI evidence continue to follow the oracle hierarchy in
`canonical_divergence_workflow.md`. Those fixtures do not vary by TeX Live
year unless their source intentionally opens distribution files.

### Annual distribution tier

Run one provisioned pdfTeX 1.40.29 executable against every selected annual
runtime tree:

1. Build a fresh reference LaTeX or pdfLaTeX format from that year's source;
   never load a historical binary format.
2. Build the corresponding Umber format from exactly the same selected tree.
3. Run a stable representative document corpus through both engines.
4. Compare DVI byte-for-byte after only the repository's established preamble
   comment normalization.
5. For PDF-specific coverage, compare the existing normalized structure and
   bounded raster evidence rather than volatile raw PDF layout.
6. Check the reference recorder output so every opened file belongs to the
   selected authenticated tree or to an explicitly staged repository input.

Extend `scripts/run-latex-parity.sh` with `--texlive YEAR` and resolve the
selected source tree and hosted distribution through the typed release policy.
The script's existing build-once format reuse and recorder-provenance checks
remain authoritative. The matrix itself is an ordinary script array or Rust
test table, not a manifest.

This tier proves parity with pdfTeX 1.40.29 while both engines consume a given
historical package tree. It does not by itself prove parity with the pdfTeX
binary shipped in that historical TeX Live release.

### Historical executable canary

Validate the single-oracle substitution without compiling many pdfTeX
versions. For each supported year, provision the upstream prebuilt pdfTeX
executable from the authenticated historic TeX Live binary archive and run a
small engine-sensitive canary set against that year's runtime tree. Compare
its normalized DVI with pdfTeX 1.40.29 using the same tree.

The binary archive is downloaded, so its upstream bytes may be pinned by the
year's acquisition lock. The conclusion of the comparison is test output; do
not commit a compatibility lock or attestation manifest. The test either
passes on the current code and inputs or reports a concrete divergent case.

If an old prebuilt binary cannot run on the host, execute it in a pinned test
container or report that annual canary as unavailable in the explicit external
test tier. Do not silently treat absence as a pass. If a genuine version
difference appears, keep that case in a small historical-engine fixture using
the repository's existing fixture workflow and state the narrower oracle
claim for that year.

## Test tiers

Routine `cargo test --tests` remains hermetic and runs no TeX Live installation
or external executable. It should cover:

- release-year parsing, supported-range completeness, and default selection;
- native and browser resolution through the single Rust authority;
- CLI conflicts and exact year-to-URL/digest resolution;
- root-digest failure, offline behavior, and cache separation by release;
- rejection of a format whose recorded distribution differs from the active
  selection; and
- deterministic publisher behavior using existing small synthetic fixtures
  for the oldest and newest supported release shapes.

The explicit distribution compatibility entry point should support a bounded
development pair and the complete release matrix:

```text
scripts/check-texlive-parity.sh --years 2020,2026
scripts/check-texlive-parity.sh --all
```

The first command is the normal focused integration check. The full 2020--2026
matrix belongs in scheduled and release testing, with authenticated downloads
and generated formats cached by their existing content identities. The script
prints one row per year with the resolved distribution, pdfTeX version,
reference-input provenance result, format result, DVI result, and optional PDF
result. Its receipt remains uncommitted test output under `target/`.

The historical executable canary is a separately named explicit command. This
keeps a missing old host binary from obscuring the stronger annual parity rows
that can run everywhere the canonical oracle runs.

## Delivery sequence

1. Add `TexliveYear`, the typed production release table, and selection tests.
2. Generalize runtime-source provisioning and publication to accept one year,
   retaining locks only for downloaded upstream inputs.
3. Publish 2020 and 2026 and add native, browser, offline, and cache-boundary
   tests.
4. Add `--texlive` to `run` and `watch`, preserving the explicit distribution
   override.
5. Bind generated-format reuse to the resolved distribution through existing
   format metadata rather than new lock files.
6. Generalize the LaTeX parity runner and land the oldest/current matrix.
7. Publish 2021 through 2025 and enable the scheduled full matrix.
8. Add the prebuilt historical-executable canary and document any observed
   version-sensitive exceptions through the existing fixture workflow.

## Acceptance criteria

- A year deterministically selects one authenticated downloaded distribution
  in native and browser builds.
- Selection never depends on a mutable remote alias or an ambient TeX tree.
- Generated formats and caches cannot cross distribution identities.
- Routine tests remain hermetic; live annual parity is explicit.
- One canonical pdfTeX build covers every annual distribution row.
- Historical prebuilt executables validate that substitution without source
  builds, and their absence cannot produce a false pass.
- No new lock or manifest describes product policy, generated local files, a
  test matrix, or a test conclusion.
