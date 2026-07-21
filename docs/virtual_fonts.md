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
