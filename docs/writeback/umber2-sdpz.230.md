# umber2-sdpz.230: Type-1 encrypted private-dictionary prelude

The representative remains the complete unmodified recent-arXiv row
`2605.22212`, entrypoint and jobname `paper-JDE-o4`. Its source is 34,138 bytes
with SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The reproduction reused the clean pdfTeX 1.40.29 reference, schema-12 Umber
format, and authenticated TeX Live 2026 distribution from the preceding row:
format SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`,
distribution-manifest SHA-256
`2595bf2f5d98f2613ef4456a2f2e3eb1f75c9c4eb324338cafa62a696f5ed20a`,
and distribution root aHash64 `49dd828f7839a1c3`. The run was offline with
`SOURCE_DATE_EPOCH=1772323200`, `FORCE_SOURCE_DATE=1`, 500,000,000 expansion
fuel, and 10,000,000 execution steps. The fixing Umber executable had SHA-256
`a22ef1a1ec81e707c397ffd69283b67544853b7ca1a10d042953bcd28a2afca5`.

Before the fix, F31's decompressed Type-1 program first differed from pdfTeX at
one-based byte 1,682, inside eexec. Decrypting both programs and disassembling
them with `t1disasm` identifies the earliest semantic divergence before any
Subrs record: Umber retained the private-dictionary line `/UniqueID 5000790
def`, while pdfTeX omitted it. Umber then retained source whitespace that
pdfTeX normalized. Only later did the programs reach the independently
different `/Subrs 4 array` (pdfTeX) and `/Subrs 18 array` (Umber) declarations.
This order rules out Subrs closure, CharStrings selection, and charstring
`lenIV` processing as the cause of the first difference.

`pdftex.web` section 799 delegates PDF font emission through
`write_fontstuff`. In pinned pdfTeX 1.40.29,
`writet1.c::t1_start_eexec` consumes the source's first four eexec seed bytes
and emits four zero seed bytes. `writet1.c::t1_read_subrs` then copies the
encrypted private-dictionary prelude through the `ptexmac.h` line buffer before
interpreting Subrs: tabs become spaces, repeated spaces collapse, trailing
spaces and empty lines disappear, line endings become LF, and `/UniqueID` is
not emitted while subsetting. Umber now applies exactly that textual rule only
up to the first `/Subrs` or `/CharStrings` declaration, then leaves binary
records to the existing bounded parser. It does not normalize arbitrary
charstring bytes.

The focused committed CMR10 control requests only `A`, `B`, and `C`. Its source
encrypted prelude contains `/UniqueID 5000793 def`, proving the old unfiltered
path is a real negative control. After subsetting, the decrypted bytes from the
four regenerated zero seed bytes up to `/Subrs` have MD5
`d14befc268a732887ef2b44c876b5abc`, exactly matching the pinned pdfTeX output,
and contain no private `/UniqueID`. A synthetic test additionally proves that
CR line endings and duplicate spaces are normalized while the `/Subrs`
declaration and following binary bytes remain untouched.

The fixed full-row PDF is 320,858 bytes with SHA-256
`e72f137d825347cb2896ba6a91d468441edc183463a4cf7f41dd2231242da1b1`.
Its normalized projection materially advances from SHA-256
`14084483f23199cc5fc65a41fb0e40a469ed4f456aa3e48f06b29305086c114a`
to `5de07a16e27eb2375f5ab328330ffae456e36f4479d1ec875fea697bd47c26a3`;
the reference remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
F31 now emits `/Length 8634 /Length1 1480 /Length2 7636 /Length3 0`, with raw
compressed-stream SHA-256
`715761782f6847a203b286198d5730fadb55c7aef6c26745fbbe6f94cefb37c3`.
The decompressed program is 9,116 bytes with SHA-256
`3bb900e2b96571c2418902134f8bcab3c2bf27b25217deb4bc423f0b1ded6b56`.
The reference remains 8,435 bytes with SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`.
The programs are now byte-exact through one-based byte 7,026. Their decrypted
eexec preludes reach `/Subrs` at zero-based byte 5,539 and share SHA-256
`bee9a353dbf4cfbf1af84f7a7e4c55e4e3ccb7db9a22971cf29b6dad93131dab`.
Thus the 1,480-byte clear segment and the newly normalized encrypted prelude
are both exact.

A fresh DVI run still passes normalized comparison against the canonical
reference. Its raw Umber DVI has SHA-256
`d35269fb139d89a93a83990e8509d02bb44a6f95ff8a7084532e66978552691d`;
the reference has
`9e709b57d0b1b684a3902a4b51e666609473dcb90f324131c628c7f03d6e3eca`.
The PDF run's `.aux` and `.out` have SHA-256
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and `a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The first remaining semantic difference is exactly the `/Subrs 4 array` versus
`/Subrs 18 array` declaration. Its diagnosis is deliberately left to the
single linked successor `umber2-sdpz.231`.
