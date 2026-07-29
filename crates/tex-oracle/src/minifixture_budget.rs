//! The regeneration-time structural budget for one committed tex82 command
//! minifixture.
//!
//! This is deliberately independent from `tools/tex-command-stream`'s
//! `AUTOMATED_MAX_SOURCES`/`AUTOMATED_MAX_SOURCE_BYTES`/`AUTOMATED_MAX_EVENTS`
//! (64 files / 64 KiB / 50,000 events). That ceiling is the differential
//! tracer's own safety net against ever loading a full document into the
//! routine `cargo test --tests` gate, checked every time the committed suite
//! is replayed. This budget is the authoring-time contract for the tex82
//! command corpus specifically: every fixture under
//! `tests/corpus/command/tex82` is now a one-or-few-source minifixture by
//! construction (`umber2-alfh.2`), so regeneration itself -- not a later test
//! run -- is where a fixture that grew back into a document gets rejected.
//! `tex-oracle` cannot depend on `tools/tex-command-stream` (the dependency
//! points the other way: the tracer already depends on `tex-oracle` for
//! [`crate::CommittedFixture`]), so the two budgets are necessarily separate
//! declarations even though they measure the same three dimensions.
//!
//! The numbers are set from the measured shape of the six fixtures produced
//! by that split: at most 8 sources, at most 2,441 source bytes, and at most
//! 3,960 committed events (the `command-transitions-v1` "spine" fixture,
//! which still covers the input-stack, scanner-status, macro, and
//! mutation/effect seams no other fixture owns). Every limit below keeps
//! comfortable headroom over that observed maximum while remaining far
//! tighter than the tracer's 64/64 KiB/50,000 ceiling.

use crate::CommittedFixture;

/// Maximum number of declared source files in one committed tex82 command
/// minifixture. The widest committed fixture (`command-transitions-v1`) uses
/// 8: its own entry source plus the transitions-child/input-recovery/EOF
/// companions that an input-stack or scanner-status transition inherently
/// needs on both sides. This leaves headroom for a similarly shaped fixture
/// without approaching a real document's file count.
pub const MINIFIXTURE_MAX_SOURCES: usize = 10;

/// Maximum combined source bytes in one committed tex82 command minifixture.
/// The widest committed source total (`expansion-macros-v1`) is 2,441 bytes.
pub const MINIFIXTURE_MAX_SOURCE_BYTES: u64 = 4 * 1024;

/// Maximum ordered committed events in one tex82 command minifixture. The
/// widest committed stream (`command-transitions-v1`) is 3,960 events.
pub const MINIFIXTURE_MAX_EVENTS: usize = 8_000;

/// Rejects a committed (or freshly bootstrapped candidate) tex82 command
/// fixture that exceeds the minifixture regeneration budget.
///
/// This is called from the regeneration path (`scripts/regen-fixtures.sh`'s
/// `--oracle tex82 ... --bootstrap-fixture` and plain fixture validation)
/// through `tex-oracle-validate --fixture`, so a fixture that grew back into
/// a small document is rejected before it is ever committed, not discovered
/// later by a routine test run.
pub fn validate_minifixture_budget(fixture: &CommittedFixture) -> Result<(), String> {
    let sources = fixture.manifest.sources.len();
    let source_bytes = fixture
        .manifest
        .sources
        .values()
        .try_fold(0_u64, |total, source| total.checked_add(source.bytes))
        .ok_or_else(|| format!("{}: source-byte total overflows u64", fixture.manifest.name))?;
    let events = fixture.stream.events.len();
    if sources <= MINIFIXTURE_MAX_SOURCES
        && source_bytes <= MINIFIXTURE_MAX_SOURCE_BYTES
        && events <= MINIFIXTURE_MAX_EVENTS
    {
        return Ok(());
    }
    Err(format!(
        "{} exceeds the tex82 minifixture regeneration budget: observed {sources} source(s), \
         {source_bytes} source byte(s), and {events} event(s); limits are \
         {MINIFIXTURE_MAX_SOURCES} source(s), {MINIFIXTURE_MAX_SOURCE_BYTES} source byte(s), and \
         {MINIFIXTURE_MAX_EVENTS} event(s). Split the fixture further instead of letting it grow \
         back into a document.",
        fixture.manifest.name
    ))
}

#[cfg(test)]
mod tests;
