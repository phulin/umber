# Semantic Testing of Command Minifixtures

The command-semantic corpus has two complementary claims. A focused projection
names the TeX operation under test and its observable order or result. The
committed reference channels compare the completed job's terminal, log, DVI,
effects, and diagnostics against independently captured TeX82/e-TeX/pdfTeX
output. A projection may be short, but it cannot substitute an internal engine
identity for a TeX result.

## Projection boundary

The 46 math and page-output cases formerly used `artifact:<hash>`
expectations, although they already had independent reference DVI files and
exact DVI channel comparisons. Those hashes came from Umber's page-plan
representation, not the reference engine. The cases now retain their command,
box, and mode assertions and assert the number of shipped pages as
`page-count:N`, with each `N` read from the _reference DVI postamble_. The
count names a TeX page-output fact; the DVI channel remains the authority for
its geometry, glyphs, specials, page order, and exact bytes. The count also
covers cases where a default output routine ships a page without an explicit
`\shipout` command observation. Their reference DVI and other channel
expectations did not change in this migration.

The page-count projection reads pages published by the corpus's focused
root-EOF run. It does not count plans allocated, retained roots, or page
hashes. The harness separately runs the complete TeX job, validates the
fragment prefix against it, and compares that complete job's DVI channel;
equal page counts alone never establish output parity.

A terminal phrase predicate reads the complete-job terminal stream, the same
stream compared to the committed reference terminal file. Final cleanup can
print after the root-EOF fragment stops. Command, macro, box, and mode
observations remain claims about the focused fragment.

The fragment and complete job must have the same command-observation prefix,
mode transitions, and published artifacts through the root-EOF boundary. A
fragment's final typed lifecycle outcome or termination observation is its
own suffix; the complete job may continue into TeX's end-job procedure and
produce a different final outcome. Earlier observation drift or a truncated
complete prefix fails execution before any channel comparison. The complete
job's status travels with its terminal, log, DVI, effects, and diagnostic
bytes and is compared with the independently captured reference status. A
clean fragment can therefore still have a fatal complete-job status after
terminal EOF. The fragment's own fatal result remains a focused projection.

Macro-call observations expose the stable call spelling and macro activation,
then the body expansion's observable command order or result. They omit the
allocator address of a definition as a command operand and retain the
independent reference channels. A definition can move in storage without
changing this claim; dropping or reordering the expansion fails.

## Negative controls

Projection tests construct a run with the same pages but changed DVI bytes and
show that the independent channel comparator rejects it. A changed page count
must fail the focused projection. Macro tests vary the definition identity
while keeping call spelling and expansion observations fixed, then remove or
reorder a body observation and require a mismatch. These checks protect the
distinction between semantic parity and incidental representation equality.

The projection migration changes fixture criteria, not TeX execution. Corpus
receipts record projection PASS changes separately from execution and channel
discrepancy changes. Reference channels are never regenerated from Umber
output.
