# Incremental HTML render sessions

Status: implementation contract for HTML patch schema 1.

This document extends [Coordinate-Identical HTML Output](html_output.md). A
render session consumes only committed `PageArtifact` values and retained
output resources. It has no access to `Universe`, engine checkpoints, mutable
stores, or partially committed execution. Cold artifact replacement and an
incremental artifact splice enter the same render-revision builder.

## Identity and equality

`RenderSessionId` is an opaque caller-scoped 128-bit value. It prevents a
revision or resource from one mounted document from being accepted by another.
It is not a content identity and is never included in a canonical digest.
Revision zero is the empty, unmounted base. Every accepted candidate receives
the next monotonic `RenderRevisionId`; rejected, cancelled, and coalesced
candidates do not consume an id.

Every render value has a typed `RenderKey`. Keys are session-scoped and consist
of a node-kind domain plus a 128-bit instance value. They are never source
pointers, allocation addresses, selectors, or executable strings. An initial
revision assigns keys deterministically from the parent key, canonical
semantic digest, and equal-value occurrence. A later revision first performs
bounded ordered matching under the same parent and reuses the old key for a
matched value. It then performs bounded unique semantic matching for moves.
Remaining values receive keys from their new parent, semantic digest,
occurrence, and target revision. Duplicate values therefore remain distinct,
and ordered matching decides their reuse without depending on hash-map order.

Pages, positioned boxes, rules, text runs, specials, fixed math containers and
events, accessibility nodes, metadata, and resources are typed canonical
values. Coordinates are signed TeX scaled-point integers. Text runs contain
the exact browser-shapeable text plus font, feature, variation, direction,
script, language, color, link, and exact anchor/baseline metadata. Browser
glyph positions, advances, ink bounds, kerning results, and shaped widths are
absent. Canonical encoding is versioned, fixed-width where practical,
length-prefixed, and independent of Rust layout and map iteration order.
SHA-256 with explicit domain separators produces subtree and revision digests.
The session id, revision id, delivery encoding, and transient resource state
are excluded.

Two mounted results are canonically equal when their document metadata,
ordered keyed tree, typed values, resource identities, accessibility tree, and
subtree digests are equal. This includes the page, box, rule, special, math,
and text-run anchor/baseline coordinates from HTML schema 1. It deliberately
excludes browser-owned within-run glyph geometry. Applying a patch must produce
the exact target canonical value and target digest.

## Patch operations

Patch schema 1 contains typed data only. A patch header binds:

- schema version and required capability bits;
- session id, base revision, and target revision;
- canonical before and after digests;
- declared operation, node, string, resource, depth, and byte counts; and
- deterministic resource additions and releases.

Operations are `Insert`, `Remove`, `Move`, and typed `Update`. Insert carries a
validated canonical node value, its parent key, and an optional following
sibling key. Remove and move name exact keys. Update carries a field mask and
typed replacement fields; it cannot carry markup, CSS text, selector text,
event handlers, scripts, or an unvalidated URL. Page replacement is represented
as removal and insertion of one changed page. No operation can replace the
document root, and unchanged subtrees emit no operation.

Operations are ordered as resource additions, deepest removals, parent-before-
child inserts, moves, leaf updates, metadata updates, and deferred resource
releases. The abstract applier validates the entire plan against an isolated
index before publication. It checks unique keys, parent existence, kind and
field compatibility, sibling membership, dependency order, declared limits,
the base digest, and the recomputed target digest. Planning returns a typed
`ResyncRequired` result if the configured comparison, operation, depth, or byte
budget cannot represent a safe patch. It never silently emits a full snapshot.

## Delivery and recovery

An initial mount and an explicit resynchronization carry a complete validated
snapshot. Ordinary revisions carry patches. One patch may be in flight per
session. The producer retains its base, target, and resource leases until it
receives an acknowledgement naming the target revision and digest.

A duplicate of the currently mounted target with the same digest is
idempotently acknowledged. A stale base, future base, missing base, wrong
session, unsupported schema/capability, unknown operation, truncation, corrupt
digest, or limit violation performs no mutation and returns a typed resync
request. A stale acknowledgement is ignored only when it exactly names an
already retired revision; conflicting acknowledgements fail the session.

Patch application is atomic at the protocol boundary. The browser first
decodes and prevalidates every value, builds new nodes in a detached fragment,
loads added fonts and assets, and simulates index changes. Only then may it
mutate the mounted tree. JavaScript exceptions during publication mark the
mount as requiring resynchronization and never acknowledge the target. The
next accepted value is a full snapshot; mixed-revision DOM is not presented as
current.

Cancellation drops an unpublished candidate and its private resource leases.
Supersession coalesces pending source edits deterministically to the newest
complete root value when no patch is in flight. Once a patch is delivered, a
new target waits for its acknowledgement or bounded timeout. Backpressure is
one delivered patch plus one coalesced candidate. Disposal rejects further
messages, revokes every owned object URL, removes installed `FontFace` values
and stylesheets, clears indexes, and releases Rust/WASM buffers.

## DOM application

The browser applier owns a `Map<RenderKey, Node>`; payload values never become
selectors. It creates elements and attributes through typed constructors
shared by initial mounting and patch insertion. It does not use `innerHTML`,
`outerHTML`, `document.write`, `eval`, payload CSS, or executable elements.
Links retain the HTML schema 1 fragment-or-HTTPS validation policy.

Untouched keys retain the same JavaScript `Node` object and are not written.
Before mutation the applier records the nearest surviving page scroll anchor,
focus, selection endpoints and direction, and observed page keys. It restores
them when the referenced nodes survive. When a focused or selected node is
removed, focus moves to the mounted document container and selection is
cleared. Accessibility ids and references are key-derived and validated as a
closed relation.

The applier performs no layout reads between its first DOM write and last DOM
write. Post-publication measurement and instrumentation happen in a later
animation frame. Counters expose operations, inserted/removed/moved/updated
nodes, encoded bytes, staged and resident resource bytes, font loads, layout
reads, apply duration, resyncs, cancellations, and coalesced candidates.

## Resource ownership and limits

Resources use their content-addressed output identity, independently of data
URL, transferred bytes, object URL, cache location, or font-family spelling.
The session tracks references held by accepted revisions, an in-flight patch,
staged DOM, the mounted DOM, and shared verified cache entries. Equal resources
are installed once. An addition is hash-verified and ready before dependent
nodes publish. A release is effective only after target acknowledgement and
the last mounted, in-flight, and staged reference disappears.

Unknown release, identity drift, conflicting bytes, failed font load, stale
acknowledgement, and budget exhaustion are typed failures. They never cause
platform-font fallback. Defaults bound pages, nodes, depth, operations,
strings, individual and aggregate resources, wire bytes, retained revisions,
in-flight work, and cumulative churn. Hosts may lower but not disable these
limits. Soak tests must demonstrate a resident-memory plateau for alternating
bounded edits and complete reclamation after disposal.

## Worked identity cases

- Prefix page insertion reuses every uniquely matched suffix page and child
  key. Page ordinals update in place; their geometry and content do not.
- Equal pages and equal lines are matched in order within their parent. Moving
  one duplicate reuses a key only when the bounded ordered/unique matching
  rules identify it unambiguously; otherwise remove plus insert is required.
- A text-only edit retains the run key when structural matching identifies the
  same run and emits one typed text update. A changed anchor or baseline is a
  geometry update. Browser-shaped width changes emit nothing.
- A page renumbering updates page metadata without replacing its subtree.
- A font-byte change has a new resource identity. The addition becomes ready,
  dependent run references update, and the old resource releases after ack.
- A stale or out-of-order patch leaves the mounted revision untouched and asks
  for a bounded full snapshot. Full snapshots are never an ordinary diff
  shortcut.

## Enforced operational budgets

Schema 1 uses the following default ceilings. Hosts may lower them through the
mount or native planner options, but cannot turn validation off.

| Quantity                                        | Default or fast-gate budget |
| ----------------------------------------------- | --------------------------: |
| Pages in a mounted snapshot                     |                      16,384 |
| Canonical nodes in a mounted snapshot           |                   1,000,000 |
| Operations in one patch                         |                     250,000 |
| Aggregate resident resource bytes               |                     256 MiB |
| One font resource                               |                      64 MiB |
| Cumulative resource churn per registry          |                       1 GiB |
| Delivered patches awaiting acknowledgement      |                           1 |
| Coalesced complete candidate behind that patch  |                           1 |
| Chromium application of 200 single-node patches |              under 1,000 ms |

The fast canonical gate performs 5,000 deterministic insert, delete, move,
replace, duplicate-content, and node-count transitions. Every transition must
apply to the exact freshly constructed target. The artifact-level gate also
rebuilds representative page artifacts from scratch, which catches drift in
lowering and identity assignment rather than only exercising the abstract
model.

The browser package gate applies 200 ordinary updates to one page in the same
mounted document. It requires exactly one operation and one replaced node per
revision, no root replacement, stable object identity for the other page and
text node, zero `MutationObserver` records under that unchanged page, bounded
application time, and complete disposal. Resource tests separately require a
resident-byte plateau under deduplicated acquisition, acknowledgement-gated
release, rollback after cancellation and font failure, cumulative-churn
rejection, and zero entries after disposal. Global reflow or protocol recovery
is observable as an explicit page operation or snapshot; it is never reported
as an ordinary local update.
