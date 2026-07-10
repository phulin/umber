//! Lazy diagnostic provenance rendering.
//!
//! The resolver is intentionally display-only. Token movement stores compact
//! `OriginId`s, while user-facing strings and source snippets are produced here
//! at diagnostic formatting boundaries.

use std::fmt::{self, Write as _};
use std::path::Path;

use crate::Universe;
use crate::input::{InputFrameSummary, SourceId};
use crate::provenance::{
    InsertedOriginKind, OriginRecord, SourceOrigin, SynthesizedOriginKind, SyntheticOriginKind,
};
use crate::source_map::SourceBacking;
use crate::token::OriginId;

const DEFAULT_TRACE_DEPTH: usize = 8;

/// Resolves raw provenance ids into user-facing diagnostic text.
pub struct ProvenanceResolver<'a> {
    universe: &'a Universe,
    trace_depth: usize,
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

    /// Renders a complete diagnostic message with optional primary origin.
    #[must_use]
    pub fn render_diagnostic(&self, message: &str, primary: Option<OriginId>) -> String {
        let mut out = String::new();
        out.push_str(message);
        out.push('\n');

        match primary {
            Some(origin) => self.render_primary_origin(&mut out, origin),
            None => out.push_str(" --> unknown origin\n"),
        }

        self.render_macro_trace(&mut out, primary);
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

    fn render_macro_trace(&self, out: &mut String, primary: Option<OriginId>) {
        let mut rendered = 0;
        for origin in self.live_macro_invocations() {
            if primary == Some(origin) {
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
        }
    }

    fn live_macro_invocations(&self) -> impl Iterator<Item = OriginId> + '_ {
        self.universe
            .input_summary()
            .frames()
            .iter()
            .rev()
            .filter_map(|frame| match frame {
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

    fn resolve_to_source(&self, origin: OriginId) -> Option<SourceOrigin> {
        if let Some(source) = self.universe.direct_source_origin(origin) {
            return Some(source);
        }
        let mut origin = origin;
        for _ in 0..self.trace_depth.saturating_add(4) {
            match self.record(origin)? {
                OriginRecord::Source(source) => return Some(source),
                OriginRecord::SourceSpan(span) => {
                    return self.universe.source_origin_at_position(span.lo());
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

    fn render_source_context(&self, out: &mut String, prefix: &str, source: SourceOrigin) {
        let display = self.source_display(source);
        let label = display.label;
        let line_number = display.line_number;
        let column = display.column;
        let _ = writeln!(out, "{prefix} {label}:{line_number}:{column}");

        let Some(line) = display.line else {
            return;
        };
        let gutter = line_number.to_string();
        let _ = writeln!(out, "  {gutter} | {line}");
        let caret_padding = caret_padding(&line, column as usize);
        let _ = writeln!(out, "  {} | {caret_padding}^", " ".repeat(gutter.len()));
    }

    fn source_label_for_record(
        &self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
    ) -> String {
        if let Some(region) = self.universe.source_region(source) {
            return match region.backing {
                SourceBacking::World(record) => self
                    .universe
                    .world()
                    .input_records()
                    .get(record.raw() as usize)
                    .map_or_else(
                        || format!("<source {}>", source.raw()),
                        |record| display_path(record.path()),
                    ),
                SourceBacking::Generated(_) => format!("<source {}>", source.raw()),
            };
        }
        let index = input_record.map_or(source.raw() as usize, |record| record.raw() as usize);
        if let Some(record) = self.universe.world().input_records().get(index) {
            return display_path(record.path());
        }
        format!("<source {}>", source.raw())
    }

    fn source_line(&self, source: SourceOrigin) -> Option<String> {
        if let Some(region) = self.universe.source_region(source.source()) {
            let bytes = self.universe.source_backing_bytes(region)?;
            let offset = usize::try_from(source.byte_offset()).ok()?;
            return physical_line_at(bytes, offset).map(|(_, _, line)| line);
        }
        let index = source
            .input_record()
            .map_or(source.source().raw() as usize, |record| {
                record.raw() as usize
            });
        let record = self.universe.world().input_records().get(index)?;
        let bytes = self.universe.world().input_content(record.hash())?;
        let text = String::from_utf8_lossy(bytes);
        line_at(&text, source.line())
    }

    fn source_display(&self, source: SourceOrigin) -> DisplaySource {
        let label = self.source_label_for_record(source.source(), source.input_record());
        if let Some(region) = self.universe.source_region(source.source())
            && source.byte_offset() <= region.byte_len
            && let Some(bytes) = self.universe.source_backing_bytes(region)
            && let Ok(offset) = usize::try_from(source.byte_offset())
            && let Some((line_number, column, line)) = physical_line_at(bytes, offset)
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

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn line_at(text: &str, line: u32) -> Option<String> {
    let index = usize::try_from(line.saturating_sub(1)).ok()?;
    text.lines()
        .nth(index)
        .map(|line| line.trim_end_matches('\r').to_owned())
}

fn physical_line_at(bytes: &[u8], offset: usize) -> Option<(u32, u32, String)> {
    if offset > bytes.len() {
        return None;
    }
    let mut starts = vec![0usize];
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' && index + 1 < bytes.len() {
            starts.push(index + 1);
        }
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
    let text = std::str::from_utf8(&bytes[line_start..content_end]).ok()?;
    let column_end = offset.min(content_end);
    let prefix = std::str::from_utf8(&bytes[line_start..column_end]).ok()?;
    Some((
        u32::try_from(line_index + 1).unwrap_or(u32::MAX),
        u32::try_from(prefix.chars().count().saturating_add(1)).unwrap_or(u32::MAX),
        text.to_owned(),
    ))
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
