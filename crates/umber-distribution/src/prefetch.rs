//! Bounded literal source hints and package-group scheduling.
//!
//! This scanner is intentionally lexical.  It recognizes only a small set of
//! literal LaTeX control sequences and never expands a macro, executes TeX,
//! or changes the engine's search semantics.

use std::collections::BTreeSet;

use crate::{FileKind, ObjectEntry};

#[cfg(test)]
mod tests;

const MAX_DEFAULT_HINTS: usize = 256;
const MAX_DEFAULT_NAME_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LiteralHintKind {
    DocumentClass,
    Package,
    Input,
    IncludeGraphics,
}

impl LiteralHintKind {
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::DocumentClass => "documentclass",
            Self::Package => "usepackage",
            Self::Input => "input",
            Self::IncludeGraphics => "includegraphics",
        }
    }

    #[must_use]
    pub const fn file_kind(self) -> FileKind {
        // TeX Live's canonical file catalogue keeps graphics and runtime
        // inputs in the tex namespace; the caller retains its richer engine
        // FileKind when admitting the resulting hint.
        FileKind::Tex
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LiteralHint {
    pub kind: LiteralHintKind,
    pub original_spelling: String,
    pub name: String,
    pub byte_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiteralHintLimits {
    pub max_hints: usize,
    pub max_name_bytes: usize,
}

impl Default for LiteralHintLimits {
    fn default() -> Self {
        Self {
            max_hints: MAX_DEFAULT_HINTS,
            max_name_bytes: MAX_DEFAULT_NAME_BYTES,
        }
    }
}

/// Extracts obvious literal file names while ignoring comments and malformed
/// command arguments.  An escaped percent is ordinary source text; an
/// unescaped percent ends the current line.
#[must_use]
pub fn extract_literal_hints(source: &str, limits: LiteralHintLimits) -> Vec<LiteralHint> {
    if limits.max_hints == 0 || limits.max_name_bytes == 0 {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut index = 0;
    while index < source.len() && output.len() < limits.max_hints {
        let Some(relative) = source[index..].find('\\') else {
            break;
        };
        index += relative;
        let slash = index;
        if in_comment(source, slash) {
            index = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            continue;
        }
        index += 1;
        let Some(command_start) = source.get(index..).and_then(|tail| {
            tail.chars()
                .next()
                .filter(|character| character.is_ascii_alphabetic())
                .map(|_| index)
        }) else {
            continue;
        };
        while index < source.len()
            && source[index..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        {
            index += source[index..]
                .chars()
                .next()
                .expect("checked command character")
                .len_utf8();
        }
        let command = &source[command_start..index];
        let kind = match command {
            "documentclass" => LiteralHintKind::DocumentClass,
            "usepackage" | "RequirePackage" => LiteralHintKind::Package,
            "input" | "include" => LiteralHintKind::Input,
            "includegraphics" => LiteralHintKind::IncludeGraphics,
            _ => continue,
        };
        let mut cursor = skip_horizontal_space(source, index);
        if matches!(
            kind,
            LiteralHintKind::DocumentClass
                | LiteralHintKind::Package
                | LiteralHintKind::IncludeGraphics
        ) && source.as_bytes().get(cursor) == Some(&b'[')
        {
            let Some(end) = balanced_bracket(source, cursor, b'[', b']') else {
                continue;
            };
            cursor = skip_horizontal_space(source, end);
        }
        let Some((argument, end)) = literal_argument(source, cursor, limits.max_name_bytes) else {
            continue;
        };
        for name in argument.split(',') {
            let name = name.trim();
            if name.is_empty()
                || name.len() > limits.max_name_bytes
                || name.chars().any(char::is_control)
            {
                continue;
            }
            output.push(LiteralHint {
                kind,
                original_spelling: name.to_owned(),
                name: name.to_owned(),
                byte_offset: slash,
            });
            if output.len() == limits.max_hints {
                break;
            }
        }
        index = end;
    }
    output
}

fn skip_horizontal_space(source: &str, mut index: usize) -> usize {
    while index < source.len() {
        let byte = source.as_bytes()[index];
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn balanced_bracket(source: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0_u32;
    let mut index = start;
    while index < source.len() {
        let byte = source.as_bytes()[index];
        if byte == b'%' && !escaped(source, index) {
            index = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            continue;
        }
        if byte == open {
            depth = depth.checked_add(1)?;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}

fn literal_argument(source: &str, start: usize, max_name_bytes: usize) -> Option<(&str, usize)> {
    if source.as_bytes().get(start) == Some(&b'{') {
        let end = balanced_bracket(source, start, b'{', b'}')?;
        let argument = source.get(start + 1..end - 1)?;
        (argument.len() <= max_name_bytes).then_some((argument, end))
    } else {
        let tail = source.get(start..)?;
        let end = tail
            .find(|character: char| character.is_whitespace() || matches!(character, '%' | '\\'))
            .map_or(source.len(), |offset| start + offset);
        let argument = source.get(start..end)?;
        (!argument.is_empty() && argument.len() <= max_name_bytes).then_some((argument, end))
    }
}

fn escaped(source: &str, index: usize) -> bool {
    let mut backslashes = 0;
    for byte in source.as_bytes()[..index].iter().rev() {
        if *byte != b'\\' {
            break;
        }
        backslashes += 1;
    }
    backslashes % 2 == 1
}

fn in_comment(source: &str, index: usize) -> bool {
    let line_start = source[..index].rfind('\n').map_or(0, |offset| offset + 1);
    let mut percent = None;
    for position in line_start..index {
        if source.as_bytes()[position] == b'%' && !escaped(source, position) {
            percent = Some(position);
        }
    }
    percent.is_some()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrefetchClass {
    SmallRuntime,
    Font,
    Image,
    Document,
    Other,
}

impl PrefetchClass {
    #[must_use]
    pub fn for_key(key: &str) -> Self {
        if key.starts_with("tfm:") || key.starts_with("font:") {
            return Self::Font;
        }
        let name = key.split_once(':').map_or(key, |(_, name)| name);
        let extension = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "eps" | "svg") {
            return Self::Image;
        }
        if matches!(extension.as_str(), "tex" | "sty" | "cls" | "def" | "ltx") {
            return Self::SmallRuntime;
        }
        if matches!(extension.as_str(), "pdf" | "ps" | "dvi") {
            Self::Document
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefetchCandidate {
    pub key: String,
    pub object: ObjectEntry,
    pub class: PrefetchClass,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrefetchBudget {
    pub max_files: usize,
    pub max_bytes: u64,
    pub max_runtime_bytes: u64,
    pub max_font_bytes: u64,
    pub max_image_bytes: u64,
    pub max_document_bytes: u64,
}

impl Default for PrefetchBudget {
    fn default() -> Self {
        Self {
            max_files: 64,
            max_bytes: 16 * 1024 * 1024,
            max_runtime_bytes: 8 * 1024 * 1024,
            max_font_bytes: 2 * 1024 * 1024,
            max_image_bytes: 2 * 1024 * 1024,
            max_document_bytes: 512 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrefetchSelection {
    pub required: Vec<PrefetchCandidate>,
    pub hints: Vec<PrefetchCandidate>,
    pub demand_bytes: u64,
    pub prefetch_bytes: u64,
}

/// Selects required records plus a bounded, deterministic small group.  The
/// required set is never dropped for speculative budget reasons; each hint
/// class has an independent ceiling so a large font/image cannot consume the
/// runtime budget.
#[must_use]
pub fn select_prefetch_group(
    required: impl IntoIterator<Item = PrefetchCandidate>,
    candidates: impl IntoIterator<Item = PrefetchCandidate>,
    budget: PrefetchBudget,
) -> PrefetchSelection {
    let mut output = PrefetchSelection::default();
    let mut seen = BTreeSet::new();
    for candidate in required {
        if !seen.insert(candidate.key.clone()) {
            continue;
        }
        output.demand_bytes = output.demand_bytes.saturating_add(candidate.object.bytes);
        output.required.push(candidate);
    }
    let mut files = output.required.len();
    let mut total = 0_u64;
    let mut by_class = [0_u64; 5];
    for mut candidate in candidates {
        if candidate.required || !seen.insert(candidate.key.clone()) {
            continue;
        }
        if files >= budget.max_files {
            break;
        }
        let class_index = candidate.class as usize;
        let class_limit = match candidate.class {
            PrefetchClass::SmallRuntime => budget.max_runtime_bytes,
            PrefetchClass::Font => budget.max_font_bytes,
            PrefetchClass::Image => budget.max_image_bytes,
            PrefetchClass::Document => budget.max_document_bytes,
            PrefetchClass::Other => budget.max_bytes,
        };
        let bytes = candidate.object.bytes;
        if bytes > class_limit.saturating_sub(by_class[class_index])
            || bytes > budget.max_bytes.saturating_sub(total)
        {
            continue;
        }
        candidate.required = false;
        by_class[class_index] = by_class[class_index].saturating_add(bytes);
        total = total.saturating_add(bytes);
        files += 1;
        output.prefetch_bytes = output.prefetch_bytes.saturating_add(bytes);
        output.hints.push(candidate);
    }
    output
}
