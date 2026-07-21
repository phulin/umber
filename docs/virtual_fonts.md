# Bounded TeX virtual-font programs

Status: architecture contract for parsing classic VF resources and handing an
immutable program to output backends.

## Authority and compatibility target

The binary grammar follows Knuth's `VFtoVP` source, version 1.4. Its sections
7--9 define the preamble and local-font definitions, short and long character
packets, legal embedded DVI commands, packet execution state, and the padded
postamble. Sections 45--51 define the reference reader's ordering and
malformed-input checks. The authoritative source is distributed by CTAN as
[`systems/knuth/dist/etc/vftovp.web`](https://ctan.org/tex-archive/systems/knuth/dist/etc).

The PDF compatibility behavior follows `pdftex.web` section 32e, "Font
processing." That section probes a VF before treating a used font as real,
loads its local fonts, validates packet commands and stack balance, and
recursively expands virtual characters. Its historical limits are a
10,000-byte packet, a 100-entry DVI stack, and 10 nested virtual-font calls.
The maintained upstream source is described on the
[`pdfTeX` project page](https://tug.org/applications/pdftex/).

## Ownership boundary

`tex-fonts` owns the byte grammar and the immutable `VfProgram`. Parsing takes
an already-acquired byte slice and performs no filesystem lookup, TFM lookup,
font-map lookup, or output. A local-font definition retains its local number,
checksum, relative scaled size, absolute design size, area, and logical name.
The parser does not decide whether those logical names resolve to TFM, VF,
outline, or PK resources.

Character packets retain their declared character code and TFM width and
normalize legal embedded DVI operations into typed `VfCommand` values. Packet
metadata records maximum push depth and the ordered local-font character
references obtained from the packet's deterministic font-selection state.
Those references are recursion edges for a later resolver; parsing does not
open referenced fonts or recursively execute them.

Resource acquisition belongs to `umber`/`umber-wasm` adapters and the shared
VFS resource protocol. Recursive lowering belongs to the PDF finalizer. DVI
output is not changed by this parser.

## PDF packet execution mapping

The detached lowerer follows `pdftex.web` section 32e, specifically the
`do_vf_packet` module and its `Do typesetting the DVI commands in virtual
character packet` fragment. A packet starts in its first declared local font;
its h/v coordinates begin at the virtual character anchor; and w/x/y/z begin
at zero. Push/pop saves all six values. The packet restores its entry h/v on
return, while a containing `set` command advances by the selected local TFM
character width and `put` does not. Packet dimensions are fix-word values
scaled by the containing virtual-font instance size. Positive-height,
positive-width rules become positioned PDF rectangles; other rules only
retain the `set_rule` advance.

The same module's `output_one_char` fragment classifies the selected local
font recursively and reaches PDF font selection only for a real leaf. Umber
therefore instantiates retained local TFM bytes at the declared size, reserves
PDF resources for real leaves, and computes glyph usage from the expanded
positioned stream. Virtual TFM names never become page font resources merely
because their source artifact contained characters.

Section 32e's `literal(s, scan_special, false)` call delegates to section
32c's `literal` procedure: non-`PDF:` specials are silently ignored;
`PDF:direct:` retains text state, `PDF:page:` ends text without moving the
origin, and an unqualified `PDF:` special uses the current packet origin.
Umber lowers those three cases to the existing typed PDF-literal operation.

The reference constants `vf_max_recursion=10` and `vf_stack_size=100` remain
defaults. Umber additionally rejects an active `(font instance, character)`
cycle explicitly and bounds aggregate executed commands, emitted operations,
and retained special bytes. All cursor, dimension, work, and output accounting
uses checked arithmetic. These finalizer-only mutations occur after engine
acceptance and do not alter committed artifacts or DVI construction.

## Typed acquisition and retry closure

The PDF-mode compile session retains a completed engine candidate before
acceptance and probes `vf:<font>.vf` for each used TFM-backed PDF font. An
authoritative negative classifies that font as real. A positive response is
parsed once, retained with both its VFS content identity and VF program
identity, and contributes required `tfm:<local>.tfm` requests. Each local TFM
then receives the same VF probe, so nested declarations reach a bounded fixed
point without executing packets.

After the VF/TFM frontier settles, real fonts drive typed `font-map`,
`font-encoding`, and `font-program` requests. The default-map optimization is
unchanged: an implicit `pdftex.map` is not requested when authoritative inline
map operations already cover every real font. These semantic wire kinds map to
the existing immutable `tex:<name>` distribution keys; native and browser
resolvers preserve the original typed request in their responses.

Positive and authoritative-negative bindings retain the shared VFS atomic
registration, conflict, byte-budget, and required-progress rules. The session
attempt limit bounds advancing closure rounds. Acquired VF programs plus exact
local-TFM bytes cross the accepted-finalization boundary. The bytes allow one
logical local TFM to be instantiated at each size declared by its containing
virtual-font instance without another host read. Recursive packet execution
then occurs only in PDF finalization and does not alter DVI construction.

This ordering maps directly to `pdftex.web` section 32e: its introductory font
processing module classifies a font by probing its VF on first PDF use; `do_vf`
and `Open vf_file` make absence the real-font fallback; and `vf_def_font` plus
`Process the font definitions` load each local TFM before packet
interpretation. Umber separates those synchronous Web2C reads into typed
resumable host requests while preserving that classification order.

## Validation and bounds

`VfLimits` makes every non-format capacity explicit. The defaults cap input,
local fonts, character packets, one packet, aggregate packet bytes, aggregate
decoded commands, aggregate special bytes, and packet push depth. The default
packet and stack limits match pdfTeX section 32e. All additions use checked
arithmetic and each owned byte allocation is preceded by the corresponding
input, packet, or special-byte bound.

The parser requires the canonical `PRE`/202 preamble, all font definitions
before packets, unique local-font and character numbers, positive local-font
scaled sizes below 2^24, complete packet commands, declared local fonts for
selection and character calls, balanced push/pop, at least one `POST`, only
`POST` padding after it, and total file length divisible by four. It rejects
unknown or forbidden DVI opcodes rather than retaining opaque executable
bytes. Long-form character codes and four-byte DVI character operands remain
32-bit in the model; the later TeX82/pdfTeX lowering boundary may reject codes
outside its 8-bit character domain.

## Verification

Hermetic synthetic fixtures cover both packet headers, local-font definitions,
typed movement/rule/font/special/character commands, recursion metadata,
truncation and malformed ordering/opcodes, and each configurable bound. Live
`vftovp` or pdfTeX execution is not part of the default Cargo correctness tier.

Session fixtures additionally cover a nested VF-to-local-TFM frontier, map,
encoding, and program acquisition, authoritative non-VF fallback, bounded
round count, and retained content identities. Native resolver fixtures prove
cold acquisition plus offline content-addressed reuse; authored browser tests
prove the same typed request survives `tex:` shard selection.
