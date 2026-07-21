# Pinned 100-document arXiv corpus

Status: corpus accounting record\
Sample identity: `scripts/pdftex-arxiv-sample-100.tsv`\
Tracking epic: `umber2-65ku`

This document records source-side cases that must not be mistaken for Umber
engine failures. The corpus inputs are retained outside Git under
`third_party/arxiv-sample-100`; the committed TSV fixes the 100 identifiers,
while the SHA-256 values below fix the source bundles used for this audit.

## Immutable archive and extracted-view boundary

The `.src` bundle under `third_party/arxiv-sample-100/archives` is the
authoritative source for each paper. Corpus identity is the tuple of its
SHA-256, the SHA-256 of the canonical JSON member inventory, and the selected
entrypoint. The member inventory sorts paths bytewise and records each regular
file's path, byte length, and SHA-256. It rejects traversal, duplicate paths,
links, and other non-file archive members.

`scripts/arxiv_corpus.py verify ARCHIVE VIEW` requires the extracted view to
contain every archive file with identical bytes and no extra file. Its
`materialize` action creates a fresh exact view, while `replace` additionally
requires a separate backup path and writes a provenance receipt containing the
old tree's complete hashed inventory. Reference runners extract the archive
into a new per-run temporary directory; generated `.aux`, `.log`, `.fls`, PDF,
and EPS-conversion outputs therefore cannot become source inputs on later runs.
The canonical view is verified again after each census child exits.

Derived reference artifacts have their own hashes and provenance under
`third_party/arxiv-sample-100/reference-artifacts`, outside both `archives` and
`sources`. In particular, the corrected `1609.01918` view has exactly the 11
archive files, archive SHA-256
`e882888d80dd010f690e2091d7690a43656568804bf7942daff1eb4658f82e24`,
member-manifest SHA-256
`e92006963a75430e0bc9573693df22a40aa07cd874f735dcf2352681ca6dbf49`,
and entrypoint `ms.tex`. Its five previously generated
`*-eps-converted-to.pdf` files and empty `msNotes.bib` are preserved, with
their exact identities and the complete pre-migration tree backup, under
`reference-artifacts/1609.01918/pre-pristine-20260721/`; none is a source
member.

## External publisher inputs

Six source bundles declare document classes or packages which are neither in
the arXiv bundle nor in Umber's pinned hosted TeX Live snapshot:

| arXiv id           | source SHA-256                                                     | selected entrypoint     | absent inputs reached from the preamble |
| ------------------ | ------------------------------------------------------------------ | ----------------------- | --------------------------------------- |
| `0809.4370`        | `b3889ad91b1bc9630d7d84c07e54644506dfe051dc7c43246d87c287a04c56e3` | `waveOpticalAnalog.tex` | `iopart.cls`                            |
| `quant-ph/0401158` | `dba130006fa9dbea3c6ae731e646b6c200a479b79548e332213682b55943e008` | `paper_final.tex`       | `iopart.cls`, then `iopams.sty`         |
| `astro-ph/9806267` | `f1bbca7f54ac24582e6210d4b1f1697f3a1b447fb08609f1cca51d98bc3f7478` | `H919.tex`              | `aa.cls`, then `astron.sty`             |
| `1607.01424`       | `7866da150c691ef81c6fb422682e7525c23a7c976d5a31d94a8768993e056540` | `main.tex`              | `svjour3.cls`                           |
| `1706.07482`       | `4a2e48e5d8c29fb42b4b3daaf026cbeedcc5cfa3bab35432d46b4f9490d924e0` | `final.tex`             | `iopart.cls`, then `iopams.sty`         |
| `hep-ph/0604209`   | `76b84d285b25c81084ea8c37f0b98622ee5e75a37164f970374994abd8365581` | `urbana1108.tex`        | `elsart.cls`, then `citesort.sty`       |

The first missing file is reproduced by each paper's
`pdftex-audit/<id>/font-audit.log`. The later preamble inputs are listed here
because adding only the first file would not close the external dependency.
The canonical hosted lookup keys for all seven distinct files are absent:

```text
tex:iopart.cls
tex:iopams.sty
tex:aa.cls
tex:astron.sty
tex:svjour3.cls
tex:elsart.cls
tex:citesort.sty
```

The hosted root is
`texlive-20260301/manifest-v3.json`, whose bytes hash to the compiled-in pin
`43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`.
For each key, SHA-256 of the UTF-8 key selects the high-byte shard (the root has
`shardBits = 8`). Fetching the root-declared `sha256-<digest>` object, checking
that object's digest, and querying `.files[$key]` returns `null`. This checks
the immutable hosted inventory itself, rather than inferring it from a local
TeX installation.

The following Bash fragment performs that complete verification. It fails if
the root pin or any shard digest differs, or if any requested key is present:

```bash
snapshot_root=/tmp/umber-texlive-20260301-manifest.json
snapshot_origin=https://assets.umber.ink/texlive/texlive-20260301
curl -fsSL "$snapshot_origin/manifest-v3.json" -o "$snapshot_root"
test "$(shasum -a 256 "$snapshot_root" | awk '{print $1}')" = \
  43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b

for snapshot_key in \
  tex:iopart.cls tex:iopams.sty tex:aa.cls tex:astron.sty \
  tex:svjour3.cls tex:elsart.cls tex:citesort.sty; do
  key_sha256=$(printf '%s' "$snapshot_key" | openssl dgst -sha256 | awk '{print $2}')
  shard_index=$((16#${key_sha256:0:2}))
  shard_digest=$(jq -r ".shards[$shard_index]" "$snapshot_root")
  shard_file="/tmp/umber-texlive-shard-$shard_index.json"
  curl -fsSL "$snapshot_origin/objects/sha256-$shard_digest" -o "$shard_file"
  test "$(shasum -a 256 "$shard_file" | awk '{print $1}')" = "$shard_digest"
  jq -e --arg key "$snapshot_key" '.files[$key] == null' "$shard_file"
done
```

The absence is consistent with TeX Live's distribution policy. TeX Live
requires freely redistributable package material and source; its maintainers
specifically declined `iopart` because it was not on CTAN and had no clear
license. Publisher pages distribute current author templates independently.
They do not establish the identity or redistribution terms of the historical
versions used by these papers. Relevant upstream records are:

- [TeX Live package contribution requirements](https://www.tug.org/texlive/pkgcontrib.html)
- [TeX Live discussion of why `iopart` could not be included](https://tug.org/pipermail/tex-live/2013-January/032906.html)
- [IOP's current author template](https://publishingsupport.iopscience.iop.org/questions/latex-template/)
- [A&A author guide for `aa.cls`](https://www.aanda.org/doc_journal/instructions/aadoc.pdf)
- [Springer Nature's current LaTeX templates](https://support.springernature.com/en/support/solutions/articles/6000250920-latex-template-package-for-article-book-submissions)
- [CTAN's current `elsarticle` class](https://ctan.org/pkg/elsarticle), which is
  a different class from the obsolete `elsart` requested by the 2006 paper

### Support decision

These six papers are **externally incomplete as pinned**. They are not clean
compile candidates for the hosted corpus and may be counted only as explicit
external-input classifications. Umber should continue to accept caller-supplied
local files through its ordinary search path, but the project must not:

- substitute `article`, `elsarticle`, `svjour`, or another modern class;
- fetch an unversioned current publisher template and claim it is the paper's
  historical input; or
- add publisher files to the hosted snapshot without a pinned byte identity,
  complete dependency closure, and verified redistribution permission.

If exact historical inputs with auditable provenance and redistribution terms
are later obtained, they can be added as a separately pinned corpus support
layer. That changes the corpus input closure; it is not an engine fix.

## Incomplete arXiv source bundle

`1307.4678` is a different class of failure. Its source archive has SHA-256
`414d5eba9285befa4e907fded2617cdc95836e39980c0d72d22822fa77689b77`.
The only TeX source in the tar archive is `paper.tex`, but that file contains:

```tex
\input{packages}
...
\input{macros}
```

at lines 28 and 229. Neither `packages.tex` nor `macros.tex` is in the archive,
and neither name resolves from the pinned distribution. The bundle contains
`paper.tex` and rendered PDF figures, so those two files cannot be recovered by
selecting a different entrypoint. Their names and use throughout the document
also show that they are author-local configuration, not standard TeX Live
packages.

### Support decision

`1307.4678` is **invalid/incomplete source as pinned**. Reconstructing either
file from later compilation errors would be document-specific guesswork. It is
an impossibility classification unless the authors' exact omitted files are
obtained and content-pinned as an explicit corpus revision.

The hosted-inventory loop above can independently check the latter claim by
substituting `tex:packages.tex tex:macros.tex` as its key list.

## Corrected entrypoint: `2002.08666`

This paper is not incomplete. Its archive SHA-256 is
`1ed7866212fa9375067cc1608fcc0aed74c3ac9da2d6e5d7cba60acc80768720`.
It contains one live document, `threshold.tex`, and several TikZ fragments.
The fragments begin with commented standalone examples such as:

```tex
% \documentclass{standalone}
```

The old profiler fallback searched for the fixed string `\documentclass`, so
it treated comments as declarations and selected the alphabetically first
fragment, `5_5_torus.tex`. That fragment correctly fails by itself because it
expects the parent to load TikZ and establish document context.

The selector now requires a live `\documentclass` declaration and retains the
existing preferred-name precedence. The reproducible result is:

```console
$ scripts/profile-pdftex-arxiv.sh check-entrypoint
entrypoint selection ok
$ scripts/profile-pdftex-arxiv.sh select-entrypoint \
    third_party/arxiv-sample-100/sources/2002.08666
third_party/arxiv-sample-100/sources/2002.08666/threshold.tex
```

`2002.08666` therefore returns to the ordinary compile census; it is not an
impossibility classification.

## Reproducing the source classifications

From a checkout with the pinned external corpus installed:

```bash
scripts/profile-pdftex-arxiv.sh check-sample
sha256sum third_party/arxiv-sample-100/archives/{0809.4370,quant-ph_0401158,astro-ph_9806267,1607.01424,1706.07482,hep-ph_0604209,1307.4678,2002.08666}.src
for id in 0809.4370 quant-ph_0401158 astro-ph_9806267 1607.01424 1706.07482 hep-ph_0604209 1307.4678 2002.08666; do
  scripts/profile-pdftex-arxiv.sh select-entrypoint \
    "third_party/arxiv-sample-100/sources/$id"
done
tar -tzf third_party/arxiv-sample-100/archives/1307.4678.src | sort
rg -n '^\\input\{(packages|macros)\}' \
  third_party/arxiv-sample-100/sources/1307.4678/paper.tex
```

The single-file `1607.01424` source is gzip data rather than a tar archive;
`scripts/profile-pdftex-arxiv.sh` deliberately expands such bundles to
`main.tex`.
