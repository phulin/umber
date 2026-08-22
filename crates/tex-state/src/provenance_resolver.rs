//! Explicitly cold provenance detachment and diagnostic presentation.
//!
//! Ordinary execution carries only generation-branded provenance coordinates.
//! A caller must present a [`ColdProvenanceDemand`] before this module resolves
//! those coordinates, reads source bytes, computes lines, or allocates strings.
//! Returned values are owned, handle-free presentation DTOs.

use core::fmt::{self, Write as _};
use unicode_width::UnicodeWidthChar;

#[cfg(test)]
use crate::durable_arena::ProvenanceId;
#[cfg(test)]
use crate::provenance::SourceOrigin;
use crate::provenance::{
    InsertedOriginKind, OriginRecord, RelatedLocationRole, SynthesizedOriginKind,
    SyntheticOriginKind,
};
use crate::token::Token;
#[cfg(test)]
use crate::universe::Universe;

#[cfg(test)]
const DEFAULT_TRACE_DEPTH: usize = 8;

/// Capability proving that a consumer explicitly requested cold provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColdProvenanceDemand {
    Diagnostic,
    RenderedSource,
}

/// Explicit cold request for one origin captured by an execution error.
#[derive(Clone, Copy, Debug)]
pub struct DiagnosticOriginRequest<'a> {
    pub demand: ColdProvenanceDemand,
    pub message: &'a str,
}

/// One generation-local live coordinate selected for cold resolution.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub struct DiagnosticProvenanceCoordinate<G> {
    pub role: Option<RelatedLocationRole>,
    pub coordinate: ProvenanceId<G>,
}

/// Explicit typed roots for one diagnostic presentation build.
#[cfg(test)]
#[derive(Debug)]
pub struct DiagnosticProvenanceRequest<'a, G> {
    pub primary: Option<ProvenanceId<G>>,
    pub related: &'a [DiagnosticProvenanceCoordinate<G>],
    pub expansion: &'a [ProvenanceId<G>],
}

/// Owned physical source range. It contains no source id or engine owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSourceLocation {
    pub path: String,
    pub start: u64,
    pub end: u64,
    pub line: u32,
    pub column: u32,
    pub excerpt: String,
}

/// Handle-free source recipe for generated or already-detached bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedGeneratedSourceSpan {
    pub logical_path: String,
    pub bytes: Vec<u8>,
    pub start: u64,
    pub end: u64,
}

/// One detached related diagnostic location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedRelatedLocation {
    pub role: RelatedLocationRole,
    pub location: Option<ResolvedSourceLocation>,
    pub summary: String,
}

/// Complete handle-free diagnostic presentation evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedDiagnosticPresentation {
    pub primary: Option<ResolvedSourceLocation>,
    pub primary_summary: String,
    pub related: Vec<DetachedRelatedLocation>,
    pub expansion: Vec<String>,
}

/// One fully detached diagnostic site and its optional generated backing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedOriginDiagnostic {
    pub rendered_site: String,
    pub resolved_source: Option<ResolvedSourceLocation>,
    pub generated_origin: Option<DetachedGeneratedSourceSpan>,
}

/// Resolver which allocates presentation only after explicit cold admission.
pub struct ProvenanceResolver<'a, G> {
    #[cfg(test)]
    universe: Option<&'a Universe<G>>,
    _demand: ColdProvenanceDemand,
    #[cfg(test)]
    trace_depth: usize,
    _brand: core::marker::PhantomData<&'a G>,
}

impl<'a, G> ProvenanceResolver<'a, G> {
    #[cfg(test)]
    #[must_use]
    pub const fn new(universe: &'a Universe<G>, demand: ColdProvenanceDemand) -> Self {
        Self {
            universe: Some(universe),
            _demand: demand,
            trace_depth: DEFAULT_TRACE_DEPTH,
            _brand: core::marker::PhantomData,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn with_trace_depth(
        universe: &'a Universe<G>,
        demand: ColdProvenanceDemand,
        trace_depth: usize,
    ) -> Self {
        Self {
            universe: Some(universe),
            _demand: demand,
            trace_depth,
            _brand: core::marker::PhantomData,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn demand(&self) -> ColdProvenanceDemand {
        self._demand
    }

    pub(crate) const fn admitted(demand: ColdProvenanceDemand) -> Self {
        Self {
            #[cfg(test)]
            universe: None,
            _demand: demand,
            #[cfg(test)]
            trace_depth: DEFAULT_TRACE_DEPTH,
            _brand: core::marker::PhantomData,
        }
    }

    pub(crate) fn detach_admitted_origin(
        &self,
        message: &str,
        record: OriginRecord,
        primary: Option<ResolvedSourceLocation>,
        generated_origin: Option<DetachedGeneratedSourceSpan>,
    ) -> DetachedOriginDiagnostic {
        let primary_summary = primary.as_ref().map_or_else(
            || record_label(record),
            |location| format!("{}:{}:{}", location.path, location.line, location.column),
        );
        let presentation = DetachedDiagnosticPresentation {
            primary,
            primary_summary,
            related: Vec::new(),
            expansion: Vec::new(),
        };
        DetachedOriginDiagnostic {
            rendered_site: render_detached_diagnostic(message, &presentation),
            resolved_source: presentation.primary,
            generated_origin,
        }
    }

    /// Detaches one live coordinate to an owned source presentation.
    #[cfg(test)]
    #[must_use]
    pub fn resolve_coordinate(
        &self,
        coordinate: ProvenanceId<G>,
    ) -> Option<ResolvedSourceLocation> {
        let record = self.record(coordinate)?;
        self.resolve_record(record)
    }

    /// Resolves an already handle-free generated-source recipe.
    #[must_use]
    pub fn resolve_generated(
        &self,
        source: &DetachedGeneratedSourceSpan,
    ) -> Option<ResolvedSourceLocation> {
        resolve_owned_bytes(
            &source.logical_path,
            &source.bytes,
            source.start,
            source.end,
        )
    }

    /// Builds complete structural/presentation evidence only at this call.
    #[cfg(test)]
    #[must_use]
    pub fn detach_diagnostic(
        &self,
        request: &DiagnosticProvenanceRequest<'_, G>,
    ) -> DetachedDiagnosticPresentation {
        let primary_record = request.primary.and_then(|id| self.record(id));
        let primary = request.primary.and_then(|id| self.resolve_coordinate(id));
        let primary_summary = primary_record.map_or_else(
            || "unknown origin".to_owned(),
            |record| self.record_summary(record),
        );
        let related = request
            .related
            .iter()
            .map(|entry| {
                let record = self.record(entry.coordinate);
                DetachedRelatedLocation {
                    role: entry.role.unwrap_or(RelatedLocationRole::SecondarySpelling),
                    location: self.resolve_coordinate(entry.coordinate),
                    summary: record.map_or_else(
                        || "unknown origin".to_owned(),
                        |record| self.record_summary(record),
                    ),
                }
            })
            .collect();
        let expansion = request
            .expansion
            .iter()
            .take(self.trace_depth)
            .map(|&id| {
                self.record(id).map_or_else(
                    || "unknown origin".to_owned(),
                    |record| self.record_summary(record),
                )
            })
            .collect();
        DetachedDiagnosticPresentation {
            primary,
            primary_summary,
            related,
            expansion,
        }
    }

    #[cfg(test)]
    fn record(&self, coordinate: ProvenanceId<G>) -> Option<OriginRecord> {
        Some(self.universe?.provenance_record(coordinate))
    }

    #[cfg(test)]
    fn resolve_record(&self, record: OriginRecord) -> Option<ResolvedSourceLocation> {
        match record {
            OriginRecord::Source(source) => self.resolve_source(source),
            OriginRecord::UnknownBootstrap
            | OriginRecord::SourceSpan(_)
            | OriginRecord::MacroInvocation(_)
            | OriginRecord::Inserted(_)
            | OriginRecord::Synthesized(_)
            | OriginRecord::Synthetic(_) => None,
        }
    }

    #[cfg(test)]
    fn resolve_source(&self, source: SourceOrigin) -> Option<ResolvedSourceLocation> {
        let universe = self.universe?;
        let record = universe.world().input_record(source.input_record()?)?;
        let bytes = universe.world().input_content(record.hash())?;
        let start = source.byte_offset();
        let start_usize = usize::try_from(start).ok()?;
        let width = utf8_scalar_len_at(bytes, start_usize).unwrap_or(1);
        let end = start.checked_add(u64::try_from(width).ok()?)?;
        resolve_owned_bytes(&record.path().to_string_lossy(), bytes, start, end)
    }

    #[cfg(test)]
    fn record_summary(&self, record: OriginRecord) -> String {
        match record {
            OriginRecord::UnknownBootstrap => "unknown origin".to_owned(),
            OriginRecord::Source(source) => self.resolve_source(source).map_or_else(
                || "source location".to_owned(),
                |location| format!("{}:{}:{}", location.path, location.line, location.column),
            ),
            OriginRecord::SourceSpan(_) => "source range".to_owned(),
            OriginRecord::MacroInvocation(_) => "macro expansion".to_owned(),
            OriginRecord::Inserted(inserted) => format!(
                "inserted {} token {}",
                inserted_kind_label(inserted.kind()),
                token_summary(inserted.token())
            ),
            OriginRecord::Synthesized(synthesized) => {
                format!(
                    "synthesized {} token",
                    synthesized_kind_label(synthesized.kind())
                )
            }
            OriginRecord::Synthetic(synthetic) => {
                format!("{} origin", synthetic_kind_label(synthetic.kind()))
            }
        }
    }
}

fn record_label(record: OriginRecord) -> String {
    match record {
        OriginRecord::UnknownBootstrap => "unknown origin".to_owned(),
        OriginRecord::Source(_) | OriginRecord::SourceSpan(_) => "source location".to_owned(),
        OriginRecord::MacroInvocation(_) => "macro expansion".to_owned(),
        OriginRecord::Inserted(inserted) => format!(
            "inserted {} token {}",
            inserted_kind_label(inserted.kind()),
            token_summary(inserted.token())
        ),
        OriginRecord::Synthesized(synthesized) => format!(
            "synthesized {} token",
            synthesized_kind_label(synthesized.kind())
        ),
        OriginRecord::Synthetic(synthetic) => {
            format!("{} origin", synthetic_kind_label(synthetic.kind()))
        }
    }
}

/// Renders a handle-free DTO without any live-state access.
#[must_use]
pub fn render_detached_diagnostic(
    message: &str,
    presentation: &DetachedDiagnosticPresentation,
) -> String {
    let mut out = String::new();
    out.push_str(message);
    out.push('\n');
    if let Some(location) = &presentation.primary {
        render_location(&mut out, " -->", location);
    } else {
        let _ = writeln!(out, " --> {}", presentation.primary_summary);
    }
    for related in &presentation.related {
        let prefix = format!("     {}", related.role.label());
        if let Some(location) = &related.location {
            render_location(&mut out, &prefix, location);
        } else {
            let _ = writeln!(out, "{prefix}: {}", related.summary);
        }
    }
    if !presentation.expansion.is_empty() {
        out.push_str("     expansion trace:\n");
        for row in &presentation.expansion {
            let _ = writeln!(out, "      {row}");
        }
    }
    out
}

fn resolve_owned_bytes(
    path: &str,
    bytes: &[u8],
    start: u64,
    end: u64,
) -> Option<ResolvedSourceLocation> {
    let start_usize = usize::try_from(start).ok()?;
    let end_usize = usize::try_from(end).ok()?;
    if start_usize > end_usize || end_usize > bytes.len() {
        return None;
    }
    let starts = line_starts(bytes);
    let index = starts
        .partition_point(|&line_start| line_start <= start_usize)
        .saturating_sub(1);
    let line_start = starts[index];
    let raw_end = bytes[line_start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(bytes.len(), |relative| line_start + relative);
    let content_end = raw_end
        .checked_sub(1)
        .filter(|&end| bytes.get(end) == Some(&b'\r'))
        .unwrap_or(raw_end);
    let excerpt = String::from_utf8_lossy(&bytes[line_start..content_end]).into_owned();
    let prefix_end = start_usize.min(content_end);
    let prefix = String::from_utf8_lossy(&bytes[line_start..prefix_end]);
    Some(ResolvedSourceLocation {
        path: path.to_owned(),
        start,
        end,
        line: u32::try_from(index + 1).unwrap_or(u32::MAX),
        column: u32::try_from(display_width(&prefix, 0) + 1).unwrap_or(u32::MAX),
        excerpt,
    })
}

fn render_location(out: &mut String, prefix: &str, location: &ResolvedSourceLocation) {
    let _ = writeln!(
        out,
        "{prefix} {}:{}:{}",
        location.path, location.line, location.column
    );
    let gutter = location.line.to_string();
    let _ = writeln!(out, "  {gutter} | {}", location.excerpt);
    let _ = writeln!(
        out,
        "  {} | {}^",
        " ".repeat(gutter.len()),
        " ".repeat(location.column.saturating_sub(1) as usize)
    );
}

fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter_map(|(index, &byte)| (byte == b'\n').then_some(index + 1)),
    );
    starts
}

#[cfg(test)]
fn utf8_scalar_len_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let width = match *bytes.get(offset)? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let end = offset.checked_add(width)?;
    let scalar = core::str::from_utf8(bytes.get(offset..end)?).ok()?;
    (scalar.chars().count() == 1).then_some(width)
}

fn display_width(text: &str, initial: usize) -> usize {
    text.chars().fold(initial, |column, ch| {
        if ch == '\t' {
            column + (8 - column % 8)
        } else {
            column + UnicodeWidthChar::width(ch).unwrap_or(0)
        }
    })
}

fn token_summary(token: Token) -> String {
    match token {
        Token::Param(slot) => format!("#{slot}"),
        _ => format!("{token:?}"),
    }
}

fn inserted_kind_label(kind: InsertedOriginKind) -> &'static str {
    match kind {
        InsertedOriginKind::EndLine => "end-line",
        InsertedOriginKind::Paragraph => "paragraph",
        InsertedOriginKind::AfterGroup => "aftergroup",
        InsertedOriginKind::AfterAssignment => "afterassignment",
        InsertedOriginKind::NoExpand => "noexpand",
        InsertedOriginKind::Unexpanded => "unexpanded",
        InsertedOriginKind::ExpandAfter => "expandafter",
        InsertedOriginKind::Unread => "unread",
        InsertedOriginKind::TokenListReplay(_) => "token-list replay",
        InsertedOriginKind::ErrorRecovery => "error-recovery",
    }
}

fn synthesized_kind_label(kind: SynthesizedOriginKind) -> &'static str {
    match kind {
        SynthesizedOriginKind::Expansion => "expansion",
        SynthesizedOriginKind::Scanner => "scanner",
        SynthesizedOriginKind::ValueRendering => "value-rendering",
        SynthesizedOriginKind::NoExpand => "noexpand",
        SynthesizedOriginKind::ErrorRecovery => "error-recovery",
    }
}

fn synthetic_kind_label(kind: SyntheticOriginKind) -> &'static str {
    match kind {
        SyntheticOriginKind::Bootstrap => "bootstrap",
        SyntheticOriginKind::Primitive => "primitive",
        SyntheticOriginKind::Format => "format",
        SyntheticOriginKind::Engine => "engine",
        SyntheticOriginKind::Test => "test",
    }
}

impl<G> fmt::Debug for ProvenanceResolver<'_, G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ProvenanceResolver");
        debug.field("demand", &self._demand);
        #[cfg(test)]
        debug.field("trace_depth", &self.trace_depth);
        debug.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
