//! Lazy diagnostic provenance rendering.
//!
//! The resolver is intentionally display-only. Token movement stores compact
//! `OriginId`s, while user-facing strings and source snippets are produced here
//! at diagnostic formatting boundaries.

use std::fmt::{self, Write as _};
use std::path::Path;
use unicode_width::UnicodeWidthChar;

use crate::Universe;
use crate::input::{InputFrameSummary, SourceId};
use crate::provenance::{
    DiagnosticSite, InsertedOriginKind, OriginRecord, SourceOrigin, SynthesizedOriginKind,
    SyntheticOriginKind,
};
use crate::source_fragments::{
    EditorLayout, FragmentStore, LayoutResolvedOrigin, direct_fragment_span, resolve_fragment_span,
};
use crate::source_map::SourceBacking;
use crate::token::OriginId;

const DEFAULT_TRACE_DEPTH: usize = 8;

/// Resolves raw provenance ids into user-facing diagnostic text.
pub struct ProvenanceResolver<'a> {
    universe: &'a Universe,
    trace_depth: usize,
}

/// Owned source range safe to return beyond the live provenance store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSourceLocation {
    pub path: String,
    pub start: u64,
    pub end: u64,
    pub line: u32,
    pub column: u32,
}

impl<'a> ProvenanceResolver<'a> {
    /// Creates a resolver with the default bounded macro trace depth.
    #[must_use]
    pub const fn new(universe: &'a Universe) -> Self {
        Self {
            universe,
            trace_depth: DEFAULT_TRACE_DEPTH,
        }
    }

    /// Creates a resolver with an explicit macro trace depth.
    #[must_use]
    pub const fn with_trace_depth(universe: &'a Universe, trace_depth: usize) -> Self {
        Self {
            universe,
            trace_depth,
        }
    }

    /// Resolves one live origin to an owned physical source range.
    #[must_use]
    pub fn resolve_origin(&self, origin: OriginId) -> Option<ResolvedSourceLocation> {
        self.resolve_origin_with_generated_path(origin, "<generated>")
    }

    /// Resolves one live origin while naming anonymous generated backing.
    #[must_use]
    pub fn resolve_origin_with_generated_path(
        &self,
        origin: OriginId,
        generated_path: &str,
    ) -> Option<ResolvedSourceLocation> {
        let resolved = self.resolve_to_source(origin)?;
        let display = self.source_display(resolved.source);
        let region = self.universe.source_region(resolved.source.source())?;
        let path = match region.backing {
            SourceBacking::World(record) => self
                .universe
                .world()
                .input_record(record)?
                .path()
                .to_string_lossy()
                .into_owned(),
            SourceBacking::Generated(_) => generated_path.to_owned(),
        };
        Some(ResolvedSourceLocation {
            path,
            start: resolved.source.byte_offset(),
            end: resolved.hi,
            line: display.line_number,
            column: display.column,
        })
    }

    /// Resolves an origin against the current editor piece table before
    /// falling through to the rollback-coupled engine source map.
    #[must_use]
    pub fn resolve_layout_origin(
        &self,
        origin: OriginId,
        fragments: &FragmentStore,
        layout: &EditorLayout,
    ) -> LayoutResolvedOrigin {
        let mut current = origin;
        for _ in 0..self.trace_depth.saturating_add(4) {
            if let Some(span) = direct_fragment_span(current, fragments) {
                return resolve_fragment_span(span, fragments, layout)
                    .unwrap_or(LayoutResolvedOrigin::Unknown);
            }
            match self.record(current) {
                Some(OriginRecord::SourceSpan(span)) => {
                    if let Some(resolved) = resolve_fragment_span(span, fragments, layout) {
                        return resolved;
                    }
                    return self
                        .resolve_origin(current)
                        .map_or(LayoutResolvedOrigin::Unknown, |_| {
                            LayoutResolvedOrigin::Foreign
                        });
                }
                Some(OriginRecord::Source(_)) => return LayoutResolvedOrigin::Foreign,
                Some(OriginRecord::MacroInvocation(invocation)) => {
                    current = invocation.invocation();
                }
                Some(OriginRecord::Inserted(inserted)) => current = inserted.parent(),
                Some(OriginRecord::Synthesized(synthesized)) => current = synthesized.parent(),
                Some(OriginRecord::UnknownBootstrap | OriginRecord::Synthetic(_)) | None => {
                    return self
                        .resolve_origin(current)
                        .map_or(LayoutResolvedOrigin::Unknown, |_| {
                            LayoutResolvedOrigin::Foreign
                        });
                }
            }
        }
        LayoutResolvedOrigin::Unknown
    }

    /// Renders a complete diagnostic message with optional primary origin.
    #[must_use]
    pub fn render_diagnostic(&self, message: &str, primary: Option<OriginId>) -> String {
        let site = DiagnosticSite::new(primary, [], self.live_macro_invocation_head());
        self.render_diagnostic_site(message, &site)
    }

    /// Renders a diagnostic from origins captured when the error was created.
    #[must_use]
    pub fn render_diagnostic_site(&self, message: &str, site: &DiagnosticSite) -> String {
        let mut out = String::new();
        out.push_str(message);
        out.push('\n');

        match site.primary_origin() {
            Some(origin) => self.render_primary_origin(&mut out, origin),
            None => out.push_str(" --> unknown origin\n"),
        }

        for related in site.related() {
            let prefix = format!("     {}", related.role().label());
            if let Some(source) = self.resolve_to_source(related.origin()) {
                self.render_source_context(&mut out, &prefix, source);
            } else {
                let _ = writeln!(out, "{prefix}: {}", self.origin_summary(related.origin()));
            }
        }
        self.render_captured_macro_trace(&mut out, site);
        out
    }

    fn render_primary_origin(&self, out: &mut String, origin: OriginId) {
        let resolved = self.resolve_to_source(origin);
        match resolved {
            Some(source) => self.render_source_context(out, " -->", source),
            None => {
                let _ = writeln!(out, " --> {}", self.origin_summary(origin));
            }
        }

        if let Some((invocation, definition)) = self.macro_invocation_pair(origin) {
            out.push_str("     macro invocation:\n");
            if let Some(source) = self.resolve_to_source(invocation) {
                self.render_source_context(out, "      invoked at", source);
            } else {
                let _ = writeln!(out, "      invoked at {}", self.origin_summary(invocation));
            }
            if let Some(source) = self.resolve_to_source(definition) {
                self.render_source_context(out, "      defined at", source);
            } else {
                let _ = writeln!(out, "      defined at {}", self.origin_summary(definition));
            }
        }
    }

    fn render_captured_macro_trace(&self, out: &mut String, site: &DiagnosticSite) {
        let mut rendered = 0;
        let mut current = site.expansion_head();
        while let Some(origin) = current {
            let parent = match self.record(origin) {
                Some(OriginRecord::MacroInvocation(invocation)) => (invocation.parent_invocation()
                    != OriginId::UNKNOWN)
                    .then_some(invocation.parent_invocation()),
                _ => None,
            };
            if site.primary_origin() == Some(origin) {
                current = parent;
                continue;
            }
            if rendered == 0 {
                out.push_str("     expansion trace:\n");
            }
            if rendered >= self.trace_depth {
                out.push_str("      ...\n");
                break;
            }

            if let Some((invocation, definition)) = self.macro_invocation_pair(origin) {
                if let Some(source) = self.resolve_to_source(invocation) {
                    self.render_source_context(out, "      invoked at", source);
                } else {
                    let _ = writeln!(out, "      invoked at {}", self.origin_summary(invocation));
                }
                if let Some(source) = self.resolve_to_source(definition) {
                    self.render_source_context(out, "      defined at", source);
                }
            } else {
                let _ = writeln!(out, "      {}", self.origin_summary(origin));
            }
            rendered += 1;
            current = parent;
        }
    }

    fn live_macro_invocation_head(&self) -> Option<OriginId> {
        self.universe
            .input_summary()
            .frames()
            .iter()
            .rev()
            .find_map(|frame| match frame {
                InputFrameSummary::TokenList {
                    macro_invocation, ..
                } if *macro_invocation != OriginId::UNKNOWN => Some(*macro_invocation),
                _ => None,
            })
    }

    fn macro_invocation_pair(&self, origin: OriginId) -> Option<(OriginId, OriginId)> {
        match self.record(origin)? {
            OriginRecord::MacroInvocation(invocation) => {
                Some((invocation.invocation(), invocation.definition_origin()))
            }
            _ => None,
        }
    }

    fn resolve_to_source(&self, origin: OriginId) -> Option<ResolvedSource> {
        if let Some(source) = self.universe.direct_source_origin(origin) {
            let hi = source
                .byte_offset()
                .checked_add(self.source_scalar_len(source).unwrap_or(1))?;
            return Some(ResolvedSource { source, hi });
        }
        let mut origin = origin;
        for _ in 0..self.trace_depth.saturating_add(4) {
            match self.record(origin)? {
                OriginRecord::Source(source) => {
                    let hi = source
                        .byte_offset()
                        .checked_add(self.source_scalar_len(source).unwrap_or(1))?;
                    return Some(ResolvedSource { source, hi });
                }
                OriginRecord::SourceSpan(span) => {
                    let source =
                        self.universe
                            .source_origin_at_position(span.lo())
                            .or_else(|| {
                                self.universe
                                    .source_region_at_position(span.lo())
                                    .map(|region| {
                                        SourceOrigin::new(region.source, region.byte_len, 0, 0)
                                    })
                            })?;
                    let region = self.universe.source_region(source.source())?;
                    let hi = span.hi().raw().checked_sub(region.start.raw())?;
                    return Some(ResolvedSource { source, hi });
                }
                OriginRecord::MacroInvocation(invocation) => {
                    origin = invocation.invocation();
                }
                OriginRecord::Inserted(inserted) => {
                    origin = inserted.parent();
                }
                OriginRecord::Synthesized(synthesized) => {
                    origin = synthesized.parent();
                }
                OriginRecord::UnknownBootstrap | OriginRecord::Synthetic(_) => return None,
            }
        }
        None
    }

    fn render_source_context(&self, out: &mut String, prefix: &str, source: ResolvedSource) {
        let display = self.source_display(source.source);
        let label = display.label;
        let line_number = display.line_number;
        let column = display.column;
        let _ = writeln!(out, "{prefix} {label}:{line_number}:{column}");

        let Some(region) = self.universe.source_region(source.source.source()) else {
            if let Some(line) = display.line {
                let gutter = line_number.to_string();
                let _ = writeln!(out, "  {gutter} | {line}");
                let padding = caret_padding(&line, column as usize);
                let _ = writeln!(out, "  {} | {padding}^", " ".repeat(gutter.len()));
            }
            return;
        };
        let Some(bytes) = self.universe.source_backing_bytes(region) else {
            return;
        };
        let Some(line_starts) = self.universe.source_line_starts(region) else {
            return;
        };
        self.render_range_lines(
            out,
            bytes,
            line_starts,
            source.source.byte_offset(),
            source.hi,
        );
    }

    fn source_scalar_len(&self, source: SourceOrigin) -> Option<u64> {
        let region = self.universe.source_region(source.source())?;
        let bytes = self.universe.source_backing_bytes(region)?;
        let offset = usize::try_from(source.byte_offset()).ok()?;
        u64::try_from(utf8_scalar_len_at(bytes, offset)?).ok()
    }

    fn render_range_lines(
        &self,
        out: &mut String,
        bytes: &[u8],
        starts: &[usize],
        lo: u64,
        hi: u64,
    ) {
        let (Ok(lo), Ok(hi)) = (usize::try_from(lo), usize::try_from(hi)) else {
            return;
        };
        if lo > bytes.len() || hi < lo || hi > bytes.len() {
            return;
        }
        let first = line_index_at(starts, lo);
        let last_probe = if hi > lo { hi - 1 } else { lo };
        let last = line_index_at(starts, last_probe.min(bytes.len()));
        self.render_one_range_line(out, bytes, starts, first, lo..hi, true);
        if last > first {
            if last > first + 1 {
                out.push_str("    | ...\n");
            }
            self.render_one_range_line(out, bytes, starts, last, lo..hi, false);
        }
    }

    fn render_one_range_line(
        &self,
        out: &mut String,
        bytes: &[u8],
        starts: &[usize],
        index: usize,
        range: std::ops::Range<usize>,
        first: bool,
    ) {
        let Some(line) = physical_line(bytes, starts, index) else {
            return;
        };
        let mark_lo = if first {
            range.start.clamp(line.start, line.content_end)
        } else {
            line.start
        };
        let mark_hi = if range.is_empty() {
            mark_lo
        } else {
            range.end.clamp(mark_lo, line.content_end)
        };
        let text = String::from_utf8_lossy(&bytes[line.start..line.content_end]);
        let prefix = String::from_utf8_lossy(&bytes[line.start..mark_lo]);
        let marked = String::from_utf8_lossy(&bytes[mark_lo..mark_hi]);
        let column = display_width(&prefix, 0);
        let width = display_width(&marked, column).saturating_sub(column).max(1);
        let number = index.saturating_add(1);
        let gutter = number.to_string();
        let _ = writeln!(out, "  {gutter} | {text}");
        let _ = writeln!(
            out,
            "  {} | {}{}",
            " ".repeat(gutter.len()),
            " ".repeat(column),
            "^".repeat(width)
        );
    }

    fn source_label_for_record(
        &self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
    ) -> String {
        if let Some(region) = self.universe.source_region(source) {
            return match region.backing {
                SourceBacking::World(record) => {
                    self.universe.world().input_record(record).map_or_else(
                        || format!("<source {}>", source.raw()),
                        |record| display_path(record.path()),
                    )
                }
                SourceBacking::Generated(_) => format!("<source {}>", source.raw()),
            };
        }
        let record = match input_record {
            Some(record) => self.universe.world().input_record(record),
            None => self
                .universe
                .world()
                .input_records()
                .get(source.raw() as usize),
        };
        if let Some(record) = record {
            return display_path(record.path());
        }
        format!("<source {}>", source.raw())
    }

    fn source_line(&self, source: SourceOrigin) -> Option<String> {
        if let Some(region) = self.universe.source_region(source.source()) {
            let bytes = self.universe.source_backing_bytes(region)?;
            let line_starts = self.universe.source_line_starts(region)?;
            let offset = usize::try_from(source.byte_offset()).ok()?;
            return physical_line_at(bytes, line_starts, offset).map(|(_, _, line)| line);
        }
        let record = match source.input_record() {
            Some(record) => self.universe.world().input_record(record)?,
            None => self
                .universe
                .world()
                .input_records()
                .get(source.source().raw() as usize)?,
        };
        let bytes = self.universe.world().input_content(record.hash())?;
        let text = String::from_utf8_lossy(bytes);
        line_at(&text, source.line())
    }

    fn source_display(&self, source: SourceOrigin) -> DisplaySource {
        let label = self.source_label_for_record(source.source(), source.input_record());
        if let Some(region) = self.universe.source_region(source.source())
            && source.byte_offset() <= region.byte_len
            && let Some(bytes) = self.universe.source_backing_bytes(region)
            && let Some(line_starts) = self.universe.source_line_starts(region)
            && let Ok(offset) = usize::try_from(source.byte_offset())
            && let Some((line_number, column, line)) = physical_line_at(bytes, line_starts, offset)
        {
            return DisplaySource {
                label,
                line_number,
                column,
                line: Some(line),
            };
        }
        DisplaySource {
            label,
            line_number: source.line().max(1),
            column: source.column().saturating_add(1).max(1),
            line: self.source_line(source),
        }
    }

    fn origin_summary(&self, origin: OriginId) -> String {
        match self.record(origin) {
            Some(OriginRecord::UnknownBootstrap) | None => "unknown origin".to_owned(),
            Some(OriginRecord::Source(source)) => {
                let label = self.source_label_for_record(source.source(), source.input_record());
                format!(
                    "{label}:{}:{}",
                    source.line().max(1),
                    source.column().saturating_add(1).max(1)
                )
            }
            Some(OriginRecord::SourceSpan(_)) => "source location".to_owned(),
            Some(OriginRecord::MacroInvocation(_)) => "macro expansion".to_owned(),
            Some(OriginRecord::Inserted(inserted)) => {
                format!(
                    "inserted {} token {:?}",
                    inserted_kind_label(inserted.kind()),
                    inserted.token()
                )
            }
            Some(OriginRecord::Synthesized(synthesized)) => {
                format!(
                    "synthesized {} token",
                    synthesized_kind_label(synthesized.kind())
                )
            }
            Some(OriginRecord::Synthetic(synthetic)) => {
                format!("{} origin", synthetic_kind_label(synthetic.kind()))
            }
        }
    }

    fn record(&self, origin: OriginId) -> Option<OriginRecord> {
        self.universe.origin_if_live(origin)
    }
}

struct DisplaySource {
    label: String,
    line_number: u32,
    column: u32,
    line: Option<String>,
}

#[derive(Clone, Copy)]
struct ResolvedSource {
    source: SourceOrigin,
    hi: u64,
}

#[derive(Clone, Copy)]
struct PhysicalLine {
    start: usize,
    content_end: usize,
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn line_at(text: &str, line: u32) -> Option<String> {
    let index = usize::try_from(line.saturating_sub(1)).ok()?;
    text.lines()
        .nth(index)
        .map(|line| line.trim_end_matches('\r').to_owned())
}

fn physical_line_at(bytes: &[u8], starts: &[usize], offset: usize) -> Option<(u32, u32, String)> {
    if offset > bytes.len() {
        return None;
    }
    let line_index = starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1);
    let line_start = starts[line_index];
    let raw_end = bytes[line_start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(bytes.len(), |relative| line_start + relative);
    let content_end = raw_end
        .checked_sub(1)
        .filter(|&end| bytes.get(end) == Some(&b'\r'))
        .unwrap_or(raw_end);
    let text = String::from_utf8_lossy(&bytes[line_start..content_end]);
    let column_end = offset.min(content_end);
    let prefix = String::from_utf8_lossy(&bytes[line_start..column_end]);
    Some((
        u32::try_from(line_index + 1).unwrap_or(u32::MAX),
        u32::try_from(display_width(&prefix, 0).saturating_add(1)).unwrap_or(u32::MAX),
        text.into_owned(),
    ))
}

fn line_index_at(starts: &[usize], offset: usize) -> usize {
    starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1)
}

fn utf8_scalar_len_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let width = match *bytes.get(offset)? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let end = offset.checked_add(width)?;
    let scalar = std::str::from_utf8(bytes.get(offset..end)?).ok()?;
    (scalar.chars().count() == 1).then_some(width)
}

fn physical_line(bytes: &[u8], starts: &[usize], index: usize) -> Option<PhysicalLine> {
    let start = *starts.get(index)?;
    let raw_end = starts
        .get(index + 1)
        .map_or(bytes.len(), |next| next.saturating_sub(1));
    let content_end = raw_end
        .checked_sub(1)
        .filter(|&end| bytes.get(end) == Some(&b'\r'))
        .unwrap_or(raw_end);
    Some(PhysicalLine { start, content_end })
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

fn caret_padding(line: &str, one_based_column: usize) -> String {
    let spaces = one_based_column.saturating_sub(1);
    let mut padding = String::new();
    for ch in line.chars().take(spaces) {
        if ch == '\t' {
            padding.push('\t');
        } else {
            padding.push(' ');
        }
    }
    padding
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

impl fmt::Debug for ProvenanceResolver<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProvenanceResolver")
            .field("trace_depth", &self.trace_depth)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
