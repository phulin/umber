# Semantic Testing of Command Minifixtures

The command-semantic corpus has two complementary claims. A focused projection
names the TeX operation under test and its observable order or result. The
committed reference channels compare the completed job's terminal, log, DVI,
effects, and diagnostics against independently captured TeX82/e-TeX/pdfTeX
output. A projection may be short, but it cannot substitute an internal engine
identity for a TeX result.

## Projection boundary

The 46 math and page-output cases with `artifact:<hash>` expectations already
have independent reference DVI files and exact DVI channel comparisons. Their
artifact hashes came from Umber's page-plan representation, not the reference
engine. Remove the hash selector and all of those expected hash values. Retain
their command, box, and mode assertions, and assert the number of shipped pages
as `page-count:N`, with each `N` read from the _reference DVI postamble_. The
count names a TeX page-output fact; the DVI channel remains the authority for
its geometry, glyphs, specials, page order, and exact bytes. The count is
especially necessary for a case where a default output routine ships a page
without an explicit `\shipout` command observation. No reference DVI or other
channel expectation changes in this migration.

The page-count projection reads pages published by the corpus's focused
root-EOF run. It does not count plans allocated, retained roots, or page
hashes. The harness separately runs the complete TeX job, validates the
fragment prefix against it, and compares that complete job's DVI channel;
equal page counts alone never establish output parity.

A terminal phrase predicate reads the complete-job terminal stream, the same
stream compared to the committed reference terminal file. Final cleanup can
print after the root-EOF fragment stops. Command, macro, box, and mode
observations remain claims about the focused fragment.

Macro-call observations likewise must not expose the allocator address of a
definition as a command operand. Project the stable call spelling and macro
activation, then the body expansion's observable command/order or result.
Retain the independent reference channels. A definition can move in storage
without changing this claim; dropping or reordering the expansion must fail.

## Negative controls

Projection tests construct a run with the same pages but changed DVI bytes and
show that the independent channel comparator rejects it. A changed page count
must fail the focused projection. Macro tests vary the definition identity
while keeping call spelling and expansion observations fixed, then remove or
reorder a body observation and require a mismatch. These checks protect the
distinction between semantic parity and incidental representation equality.

The migration changes fixture criteria, not TeX execution. Record projection
PASS changes separately from execution and channel discrepancy changes in the
manual corpus census. Do not regenerate reference channels from Umber output.
