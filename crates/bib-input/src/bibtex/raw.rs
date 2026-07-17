use std::collections::BTreeMap;

use bib_unicode::decode_legacy;

use super::{BibTexDiagnostic, BibTexDiagnosticKind, BibTexLimits, BibTexOptions, diagnostic};

/// A byte-oriented location in a decoded BibTeX datasource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawBibLocation {
    byte_start: usize,
    byte_end: usize,
    line: usize,
    column: usize,
}

impl RawBibLocation {
    #[must_use]
    pub const fn byte_start(self) -> usize {
        self.byte_start
    }
    #[must_use]
    pub const fn byte_end(self) -> usize {
        self.byte_end
    }
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }
    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }
}

/// An identifier retaining its source spelling and case-folded lookup form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibIdentifier {
    source: String,
    folded: String,
    location: RawBibLocation,
}

impl RawBibIdentifier {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub fn folded(&self) -> &str {
        &self.folded
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// A TeX control sequence retained inside a raw literal part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibControlSequence {
    source: String,
    location: RawBibLocation,
}

impl RawBibControlSequence {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// Literal source text, including brace and control-sequence syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibText {
    source: String,
    location: RawBibLocation,
    controls: Vec<RawBibControlSequence>,
}

impl RawBibText {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
    #[must_use]
    pub fn control_sequences(&self) -> &[RawBibControlSequence] {
        &self.controls
    }
}

/// One unexpanded component of a BibTeX value expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawBibValuePart {
    Braced(RawBibText),
    Quoted(RawBibText),
    Number(RawBibText),
    Macro(RawBibIdentifier),
}

impl RawBibValuePart {
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        match self {
            Self::Braced(value) | Self::Quoted(value) | Self::Number(value) => value.location(),
            Self::Macro(value) => value.location(),
        }
    }
}

/// A concatenated BibTeX value expression before macro expansion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibValue {
    parts: Vec<RawBibValuePart>,
    location: RawBibLocation,
}

impl RawBibValue {
    #[must_use]
    pub fn parts(&self) -> &[RawBibValuePart] {
        &self.parts
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// A raw field assignment, including duplicate assignments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibField {
    name: RawBibIdentifier,
    value: RawBibValue,
    location: RawBibLocation,
}

impl RawBibField {
    #[must_use]
    pub fn name(&self) -> &RawBibIdentifier {
        &self.name
    }
    #[must_use]
    pub fn value(&self) -> &RawBibValue {
        &self.value
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// A raw database entry, retained even when its key collides with another entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibEntry {
    entry_type: RawBibIdentifier,
    key: RawBibIdentifier,
    fields: Vec<RawBibField>,
    location: RawBibLocation,
}

impl RawBibEntry {
    #[must_use]
    pub fn entry_type(&self) -> &RawBibIdentifier {
        &self.entry_type
    }
    #[must_use]
    pub fn key(&self) -> &RawBibIdentifier {
        &self.key
    }
    #[must_use]
    pub fn fields(&self) -> &[RawBibField] {
        &self.fields
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibStringMacro {
    name: RawBibIdentifier,
    value: RawBibValue,
    location: RawBibLocation,
}

impl RawBibStringMacro {
    #[must_use]
    pub fn name(&self) -> &RawBibIdentifier {
        &self.name
    }
    #[must_use]
    pub fn value(&self) -> &RawBibValue {
        &self.value
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibPreamble {
    value: RawBibValue,
    location: RawBibLocation,
}

impl RawBibPreamble {
    #[must_use]
    pub fn value(&self) -> &RawBibValue {
        &self.value
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibComment {
    value: RawBibText,
    location: RawBibLocation,
}

impl RawBibComment {
    #[must_use]
    pub fn value(&self) -> &RawBibText {
        &self.value
    }
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// A parser recovery event retained in record order for classic diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibRecovery {
    location: RawBibLocation,
}

impl RawBibRecovery {
    #[must_use]
    pub const fn location(&self) -> RawBibLocation {
        self.location
    }
}

/// A source-ordered raw datasource record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawBibRecord {
    Entry(RawBibEntry),
    String(RawBibStringMacro),
    Preamble(RawBibPreamble),
    Comment(RawBibComment),
    Recovery(RawBibRecovery),
}

/// Lossless BibTeX syntax retained before backend-specific conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBibDatabase {
    records: Vec<RawBibRecord>,
    diagnostics: Vec<BibTexDiagnostic>,
    options: BibTexOptions,
}

impl RawBibDatabase {
    #[must_use]
    pub fn records(&self) -> &[RawBibRecord] {
        &self.records
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[BibTexDiagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub const fn options(&self) -> BibTexOptions {
        self.options
    }

    /// Returns the raw view that later classic `READ` processing consumes.
    #[must_use]
    pub const fn classic(&self) -> RawBibClassicSource<'_> {
        RawBibClassicSource { database: self }
    }
}

/// Borrowed classic-facing view; it deliberately performs no Biber conversion.
#[derive(Clone, Copy, Debug)]
pub struct RawBibClassicSource<'a> {
    database: &'a RawBibDatabase,
}

impl<'a> RawBibClassicSource<'a> {
    #[must_use]
    pub fn records(self) -> &'a [RawBibRecord] {
        self.database.records()
    }
    #[must_use]
    pub fn diagnostics(self) -> &'a [BibTexDiagnostic] {
        self.database.diagnostics()
    }
}

/// Parses a datasource without expanding macros or normalizing value syntax.
#[must_use]
pub fn parse_raw_bibtex_bytes(bytes: &[u8], options: BibTexOptions) -> RawBibDatabase {
    if bytes.len() > options.limits.max_bytes {
        return RawBibDatabase {
            records: Vec::new(),
            diagnostics: vec![diagnostic(
                BibTexDiagnosticKind::Limit,
                0,
                "datasource byte limit exceeded",
            )],
            options,
        };
    }
    let (input, offsets) = match decoded_source(bytes, options) {
        Some(value) => value,
        None => {
            return RawBibDatabase {
                records: Vec::new(),
                diagnostics: vec![diagnostic(
                    BibTexDiagnosticKind::Encoding,
                    0,
                    "datasource cannot be decoded with the selected encoding",
                )],
                options,
            };
        }
    };
    Parser::new(&input, offsets, bytes, options).parse()
}

fn decoded_source(bytes: &[u8], options: BibTexOptions) -> Option<(String, Vec<usize>)> {
    if matches!(options.encoding, bib_unicode::LegacyEncoding::Utf8) {
        let text = decode_legacy(bytes, options.encoding).ok()?;
        return Some((text, (0..=bytes.len()).collect()));
    }
    let mut text = String::new();
    let mut offsets = Vec::new();
    for (offset, byte) in bytes.iter().copied().enumerate() {
        let value = decode_legacy(&[byte], options.encoding).ok()?;
        offsets.extend(std::iter::repeat_n(offset, value.len()));
        text.push_str(&value);
    }
    offsets.push(bytes.len());
    Some((text, offsets))
}

struct Parser<'a> {
    input: &'a str,
    offsets: Vec<usize>,
    line_starts: Vec<usize>,
    at: usize,
    limits: BibTexLimits,
    work: usize,
    source: RawBibDatabase,
    keys: BTreeMap<String, String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, offsets: Vec<usize>, bytes: &[u8], options: BibTexOptions) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            bytes
                .iter()
                .enumerate()
                .filter_map(|(at, byte)| (*byte == b'\n').then_some(at + 1)),
        );
        Self {
            input,
            offsets,
            line_starts,
            at: 0,
            limits: options.limits,
            work: 0,
            source: RawBibDatabase {
                records: Vec::new(),
                diagnostics: Vec::new(),
                options,
            },
            keys: BTreeMap::new(),
        }
    }

    fn parse(mut self) -> RawBibDatabase {
        while self.at < self.input.len() && self.work < self.limits.max_work {
            if !self.seek_record() {
                break;
            }
            let start = self.at - 1;
            self.space();
            let kind_start = self.at;
            let kind = self.identifier();
            let kind_identifier = self.identifier_at(kind_start, self.at, kind);
            self.space();
            let Some(open) = self.peek() else { break };
            if open != b'{' && open != b'(' {
                self.error(
                    BibTexDiagnosticKind::Syntax,
                    start,
                    "expected `{` or `(` after entry type",
                );
                self.recovery(start);
                continue;
            }
            self.at += 1;
            let close = if open == b'{' { b'}' } else { b')' };
            match kind_identifier.folded.as_str() {
                "comment" => self.parse_comment(start, close),
                "preamble" => self.parse_preamble(start, close),
                "string" => self.parse_string(start, close),
                "" => self.recover(close, start),
                _ => self.parse_entry(start, close, kind_identifier),
            }
        }
        if self.work >= self.limits.max_work {
            self.error(
                BibTexDiagnosticKind::Limit,
                self.at,
                "parser work limit exceeded",
            );
        }
        self.source
    }

    fn parse_comment(&mut self, start: usize, close: u8) {
        let value_start = self.at;
        let open = if close == b'}' { b'{' } else { b'(' };
        let Some(end) = self.skip_balanced(open, close) else {
            self.recover(close, start);
            return;
        };
        let value = self.raw_text(value_start, end.saturating_sub(1));
        self.source
            .records
            .push(RawBibRecord::Comment(RawBibComment {
                value,
                location: self.location(start, end),
            }));
    }

    fn parse_preamble(&mut self, start: usize, close: u8) {
        let Some(value) = self.value(close, 0) else {
            self.recover(close, start);
            return;
        };
        let Some(end) = self.finish_record(close, start) else {
            return;
        };
        self.source
            .records
            .push(RawBibRecord::Preamble(RawBibPreamble {
                value,
                location: self.location(start, end),
            }));
    }

    fn parse_string(&mut self, start: usize, close: u8) {
        self.space();
        let name_start = self.at;
        let name = self.identifier();
        let name = self.identifier_at(name_start, self.at, name);
        self.space();
        if name.source.is_empty() || !self.eat(b'=') {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "malformed string macro",
            );
            self.recover(close, start);
            return;
        }
        if self
            .source
            .records
            .iter()
            .filter(|record| matches!(record, RawBibRecord::String(_)))
            .count()
            .saturating_add(12)
            >= self.limits.max_macros
        {
            self.error(BibTexDiagnosticKind::Limit, start, "macro limit exceeded");
            self.recover(close, start);
            return;
        }
        let Some(value) = self.value(close, 0) else {
            self.recover(close, start);
            return;
        };
        let Some(end) = self.finish_record(close, start) else {
            return;
        };
        self.source
            .records
            .push(RawBibRecord::String(RawBibStringMacro {
                name,
                value,
                location: self.location(start, end),
            }));
    }

    fn parse_entry(&mut self, start: usize, close: u8, entry_type: RawBibIdentifier) {
        if self
            .source
            .records
            .iter()
            .filter(|record| matches!(record, RawBibRecord::Entry(_)))
            .count()
            >= self.limits.max_entries
        {
            self.error(BibTexDiagnosticKind::Limit, start, "entry limit exceeded");
            self.recover(close, start);
            return;
        }
        self.space();
        let key_start = self.at;
        while self
            .peek()
            .is_some_and(|byte| byte != b',' && byte != close)
        {
            self.at += 1;
        }
        let key_end = self.at;
        let source = self.input[key_start..key_end].trim();
        let leading = self.input[key_start..key_end].len()
            - self.input[key_start..key_end].trim_start().len();
        let key = self.identifier_at(
            key_start + leading,
            key_start + leading + source.len(),
            source.to_owned(),
        );
        if key.source.is_empty() || !self.eat(b',') {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "entry has no key or field separator",
            );
            self.recover(close, start);
            return;
        }
        if let Some(previous) = self.keys.get(&key.folded) {
            let kind = if previous == &key.source {
                BibTexDiagnosticKind::DuplicateEntry
            } else {
                BibTexDiagnosticKind::CaseCollision
            };
            self.error(
                kind,
                key_start,
                format!(
                    "entry key `{}` collides with earlier `{previous}`",
                    key.source
                ),
            );
        } else {
            self.keys.insert(key.folded.clone(), key.source.clone());
        }
        let mut fields = Vec::new();
        loop {
            self.space();
            if self.eat(close) {
                break;
            }
            if self.eat(b',') {
                continue;
            }
            let field_start = self.at;
            let name = self.identifier();
            let name = self.identifier_at(field_start, self.at, name);
            self.space();
            if name.source.is_empty() || !self.eat(b'=') {
                self.error(
                    BibTexDiagnosticKind::Syntax,
                    field_start,
                    "malformed field assignment",
                );
                self.recover(close, start);
                return;
            }
            if fields.len() >= self.limits.max_fields_per_entry {
                self.error(
                    BibTexDiagnosticKind::Limit,
                    field_start,
                    "field limit exceeded",
                );
                self.recover(close, start);
                return;
            }
            let Some(value) = self.value(close, 0) else {
                self.recover(close, start);
                return;
            };
            if fields
                .iter()
                .any(|field: &RawBibField| field.name.folded == name.folded)
            {
                self.error(
                    BibTexDiagnosticKind::DuplicateField,
                    field_start,
                    format!(
                        "duplicate field `{}` in entry `{}`",
                        name.source, key.source
                    ),
                );
            }
            let location = self.location(field_start, self.at);
            fields.push(RawBibField {
                name,
                value,
                location,
            });
            self.space();
            if self.eat(close) {
                break;
            }
            if !self.eat(b',') {
                self.error(
                    BibTexDiagnosticKind::Syntax,
                    self.at,
                    "expected comma after field",
                );
                self.recover(close, start);
                return;
            }
        }
        let location = self.location(start, self.at);
        self.source.records.push(RawBibRecord::Entry(RawBibEntry {
            entry_type,
            key,
            fields,
            location,
        }));
    }

    fn value(&mut self, close: u8, depth: usize) -> Option<RawBibValue> {
        if depth > self.limits.max_nesting {
            self.error(
                BibTexDiagnosticKind::Limit,
                self.at,
                "value nesting limit exceeded",
            );
            return None;
        }
        self.space();
        let start = self.at;
        let mut parts = Vec::new();
        let mut size = 0usize;
        loop {
            self.space();
            let part = match self.peek()? {
                b'{' => RawBibValuePart::Braced(self.braced(depth + 1)?),
                b'"' => RawBibValuePart::Quoted(self.quoted(depth + 1)?),
                byte if byte.is_ascii_digit() => RawBibValuePart::Number(self.number()),
                _ => {
                    let offset = self.at;
                    let name = self.identifier();
                    if name.is_empty() {
                        self.error(BibTexDiagnosticKind::Syntax, offset, "expected value");
                        return None;
                    }
                    RawBibValuePart::Macro(self.identifier_at(offset, self.at, name))
                }
            };
            size = size.saturating_add(part_size(&part));
            if size > self.limits.max_value_bytes {
                self.error(
                    BibTexDiagnosticKind::Limit,
                    self.at,
                    "value byte limit exceeded",
                );
                return None;
            }
            parts.push(part);
            self.space();
            if !self.eat(b'#') {
                break;
            }
        }
        if self.peek() == Some(close) || self.peek() == Some(b',') {
            Some(RawBibValue {
                parts,
                location: self.location(start, self.at),
            })
        } else {
            self.error(
                BibTexDiagnosticKind::Syntax,
                self.at,
                "unexpected token after value",
            );
            None
        }
    }

    fn braced(&mut self, depth: usize) -> Option<RawBibText> {
        self.at += 1;
        let start = self.at;
        let mut nesting = 1usize;
        let mut escaped = false;
        let mut recovery = None;
        while let Some(byte) = self.peek() {
            if byte == b'@' && self.is_line_record_boundary(self.at) {
                recovery = Some(self.at);
            }
            self.work += 1;
            if self.work >= self.limits.max_work {
                return None;
            }
            self.at += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'{' {
                nesting += 1;
                if depth + nesting > self.limits.max_nesting {
                    self.error(
                        BibTexDiagnosticKind::Limit,
                        self.at,
                        "brace nesting limit exceeded",
                    );
                    return None;
                }
            }
            if byte == b'}' {
                nesting -= 1;
                if nesting == 0 {
                    return Some(self.raw_text(start, self.at - 1));
                }
            }
        }
        if let Some(offset) = recovery {
            self.at = offset;
        }
        self.error(
            BibTexDiagnosticKind::Syntax,
            start,
            "unterminated braced value",
        );
        None
    }

    fn quoted(&mut self, depth: usize) -> Option<RawBibText> {
        self.at += 1;
        let start = self.at;
        let mut braces = 0usize;
        let mut escaped = false;
        let mut recovery = None;
        while let Some(byte) = self.peek() {
            if byte == b'@' && self.is_line_record_boundary(self.at) {
                recovery = Some(self.at);
            }
            self.at += 1;
            self.work += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'{' {
                braces += 1;
                if depth + braces > self.limits.max_nesting {
                    self.error(
                        BibTexDiagnosticKind::Limit,
                        self.at,
                        "quote brace nesting limit exceeded",
                    );
                    return None;
                }
            }
            if byte == b'}' && braces > 0 {
                braces -= 1;
            }
            if byte == b'"' && braces == 0 {
                return Some(self.raw_text(start, self.at - 1));
            }
        }
        if let Some(offset) = recovery {
            self.at = offset;
        }
        self.error(
            BibTexDiagnosticKind::Syntax,
            start,
            "unterminated quoted value",
        );
        None
    }

    fn number(&mut self) -> RawBibText {
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        self.raw_text(start, self.at)
    }

    fn identifier(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(is_identifier_byte) {
            self.at += 1;
        }
        self.input[start..self.at].to_owned()
    }

    fn finish_record(&mut self, close: u8, start: usize) -> Option<usize> {
        self.space();
        if self.eat(b',') {
            self.space();
        }
        if self.eat(close) {
            Some(self.at)
        } else {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "record has trailing content",
            );
            self.recover(close, start);
            None
        }
    }

    fn recover(&mut self, close: u8, start: usize) {
        let recovery_start = self.at;
        let mut braces = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
            let offset = self.at;
            self.at += 1;
            self.work += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'"' {
                quoted = !quoted;
                continue;
            }
            if quoted {
                continue;
            }
            if byte == b'{' {
                braces += 1;
            }
            if byte == b'}' && braces > 0 {
                braces -= 1;
            }
            let boundary = byte == b'@' && self.is_line_record_boundary(offset);
            if (braces == 0 && byte == close) || boundary {
                if byte == b'@' {
                    self.at -= 1;
                }
                break;
            }
            if self.work >= self.limits.max_work {
                break;
            }
        }
        self.source
            .records
            .push(RawBibRecord::Recovery(RawBibRecovery {
                location: self.location(start.min(recovery_start), self.at),
            }));
    }

    fn recovery(&mut self, start: usize) {
        self.source
            .records
            .push(RawBibRecord::Recovery(RawBibRecovery {
                location: self.location(start, self.at),
            }));
    }

    fn seek_record(&mut self) -> bool {
        while let Some(byte) = self.peek() {
            self.at += 1;
            self.work = self.work.saturating_add(1);
            if byte == b'%' {
                while let Some(comment_byte) = self.peek() {
                    self.at += 1;
                    self.work = self.work.saturating_add(1);
                    if comment_byte == b'\n' || self.work >= self.limits.max_work {
                        break;
                    }
                }
            } else if byte == b'@' {
                return true;
            }
            if self.work >= self.limits.max_work {
                return false;
            }
        }
        false
    }

    fn is_line_record_boundary(&self, offset: usize) -> bool {
        self.input[..offset]
            .rsplit_once('\n')
            .map_or(offset == 0, |(_, prefix)| prefix.trim().is_empty())
    }

    fn skip_balanced(&mut self, open: u8, close: u8) -> Option<usize> {
        let mut nesting = 1usize;
        while let Some(byte) = self.peek() {
            self.at += 1;
            self.work += 1;
            if byte == open {
                nesting += 1;
            }
            if byte == close {
                nesting -= 1;
                if nesting == 0 {
                    return Some(self.at);
                }
            }
            if self.work >= self.limits.max_work {
                break;
            }
        }
        self.error(
            BibTexDiagnosticKind::Syntax,
            self.at,
            "unterminated comment record",
        );
        None
    }

    fn raw_text(&self, start: usize, end: usize) -> RawBibText {
        let source = self.input[start..end].to_owned();
        let mut controls = Vec::new();
        let bytes = source.as_bytes();
        let mut at = 0usize;
        while at < bytes.len() {
            if bytes[at] != b'\\' {
                at += 1;
                continue;
            }
            let control_start = at;
            at += 1;
            if at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                    at += 1;
                }
            } else if at < bytes.len() {
                at += 1;
            }
            controls.push(RawBibControlSequence {
                source: source[control_start..at].to_owned(),
                location: self.location(start + control_start, start + at),
            });
        }
        RawBibText {
            source,
            location: self.location(start, end),
            controls,
        }
    }

    fn identifier_at(&self, start: usize, end: usize, source: String) -> RawBibIdentifier {
        RawBibIdentifier {
            folded: source.to_ascii_lowercase(),
            source,
            location: self.location(start, end),
        }
    }

    fn location(&self, start: usize, end: usize) -> RawBibLocation {
        let byte_start = self.raw_offset(start);
        let byte_end = self.raw_offset(end);
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= byte_start)
            .saturating_sub(1);
        RawBibLocation {
            byte_start,
            byte_end,
            line: line_index + 1,
            column: byte_start - self.line_starts[line_index] + 1,
        }
    }

    fn raw_offset(&self, offset: usize) -> usize {
        self.offsets.get(offset).copied().unwrap_or_else(|| {
            *self
                .offsets
                .last()
                .expect("decoded source always has an end offset")
        })
    }

    fn error(&mut self, kind: BibTexDiagnosticKind, offset: usize, message: impl Into<String>) {
        if self.source.diagnostics.len() < self.limits.max_diagnostics {
            let offset = self.raw_offset(offset);
            self.source
                .diagnostics
                .push(diagnostic(kind, offset, message));
        }
    }
    fn space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.at += 1;
        }
    }
    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.at).copied()
    }
}

fn part_size(part: &RawBibValuePart) -> usize {
    match part {
        RawBibValuePart::Braced(value)
        | RawBibValuePart::Quoted(value)
        | RawBibValuePart::Number(value) => value.source.len(),
        RawBibValuePart::Macro(value) => value.source.len(),
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace()
        && !matches!(
            byte,
            b'=' | b',' | b'{' | b'}' | b'(' | b')' | b'"' | b'#' | b'@'
        )
}
