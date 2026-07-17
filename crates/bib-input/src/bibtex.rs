use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bib_unicode::{LegacyEncoding, RecodeSet, TexRecoder, decode_legacy};
use umber_vfs::{FileContentId, VfsSnapshot, VirtualPath};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BibTexLimits {
    pub max_bytes: usize,
    pub max_entries: usize,
    pub max_fields_per_entry: usize,
    pub max_macros: usize,
    pub max_value_bytes: usize,
    pub max_nesting: usize,
    pub max_work: usize,
    pub max_diagnostics: usize,
}

impl Default for BibTexLimits {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 100_000,
            max_fields_per_entry: 1_000,
            max_macros: 10_000,
            max_value_bytes: 1024 * 1024,
            max_nesting: 256,
            max_work: 64 * 1024 * 1024,
            max_diagnostics: 1_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BibTexOptions {
    pub encoding: LegacyEncoding,
    pub decode: RecodeSet,
    pub limits: BibTexLimits,
}

impl Default for BibTexOptions {
    fn default() -> Self {
        Self {
            encoding: LegacyEncoding::Utf8,
            decode: RecodeSet::Base,
            limits: BibTexLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibTexDiagnosticKind {
    Encoding,
    Syntax,
    UndefinedMacro,
    DuplicateEntry,
    CaseCollision,
    DuplicateField,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexDiagnostic {
    pub kind: BibTexDiagnosticKind,
    pub offset: usize,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawName {
    value: String,
}

impl RawName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexField {
    name: String,
    value: String,
    names: Option<Vec<RawName>>,
}

impl BibTexField {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
    #[must_use]
    pub fn raw_names(&self) -> Option<&[RawName]> {
        self.names.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexEntry {
    key: String,
    entry_type: String,
    fields: Vec<BibTexField>,
}

impl BibTexEntry {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn entry_type(&self) -> &str {
        &self.entry_type
    }
    #[must_use]
    pub fn fields(&self) -> &[BibTexField] {
        &self.fields
    }
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&BibTexField> {
        self.fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexPreamble(String);

impl BibTexPreamble {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexSource {
    entries: Vec<BibTexEntry>,
    preambles: Vec<BibTexPreamble>,
    macros: BTreeMap<String, String>,
    diagnostics: Vec<BibTexDiagnostic>,
}

impl BibTexSource {
    #[must_use]
    pub fn entries(&self) -> &[BibTexEntry] {
        &self.entries
    }
    #[must_use]
    pub fn preambles(&self) -> &[BibTexPreamble] {
        &self.preambles
    }
    #[must_use]
    pub fn macros(&self) -> &BTreeMap<String, String> {
        &self.macros
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[BibTexDiagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&BibTexEntry> {
        self.entries
            .iter()
            .find(|entry| entry.key.eq_ignore_ascii_case(key))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    content: FileContentId,
    encoding: u8,
    decode: u8,
    limits: BibTexLimits,
}

#[derive(Debug, Default)]
pub struct BibTexCache {
    values: HashMap<CacheKey, Arc<BibTexSource>>,
}

impl BibTexCache {
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn parse(
        &mut self,
        snapshot: &VfsSnapshot,
        path: &VirtualPath,
        options: BibTexOptions,
    ) -> Result<Arc<BibTexSource>, BibTexDiagnostic> {
        let file = snapshot
            .get(path)
            .map_err(|error| {
                diagnostic(
                    BibTexDiagnosticKind::Syntax,
                    0,
                    format!("cannot read `{path}`: {error}"),
                )
            })?
            .ok_or_else(|| {
                diagnostic(
                    BibTexDiagnosticKind::Syntax,
                    0,
                    format!("datasource `{path}` is missing"),
                )
            })?;
        let key = CacheKey::new(file.content_id(), options);
        if let Some(source) = self.values.get(&key) {
            return Ok(Arc::clone(source));
        }
        let source = Arc::new(parse_bibtex_bytes(file.bytes(), options));
        self.values.insert(key, Arc::clone(&source));
        Ok(source)
    }
}

impl CacheKey {
    fn new(content: FileContentId, options: BibTexOptions) -> Self {
        Self {
            content,
            encoding: match options.encoding {
                LegacyEncoding::Utf8 => 0,
                LegacyEncoding::Latin1 => 1,
                LegacyEncoding::Latin2 => 2,
                LegacyEncoding::Latin3 => 3,
                LegacyEncoding::MacRoman => 4,
            },
            decode: match options.decode {
                RecodeSet::Null => 0,
                RecodeSet::Base => 1,
                RecodeSet::Full => 2,
            },
            limits: options.limits,
        }
    }
}

pub fn parse_bibtex(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    options: BibTexOptions,
) -> Result<BibTexSource, BibTexDiagnostic> {
    let file = snapshot
        .get(path)
        .map_err(|error| {
            diagnostic(
                BibTexDiagnosticKind::Syntax,
                0,
                format!("cannot read `{path}`: {error}"),
            )
        })?
        .ok_or_else(|| {
            diagnostic(
                BibTexDiagnosticKind::Syntax,
                0,
                format!("datasource `{path}` is missing"),
            )
        })?;
    Ok(parse_bibtex_bytes(file.bytes(), options))
}

#[must_use]
pub fn parse_bibtex_bytes(bytes: &[u8], options: BibTexOptions) -> BibTexSource {
    if bytes.len() > options.limits.max_bytes {
        return BibTexSource {
            entries: Vec::new(),
            preambles: Vec::new(),
            macros: month_macros(),
            diagnostics: vec![diagnostic(
                BibTexDiagnosticKind::Limit,
                0,
                "datasource byte limit exceeded",
            )],
        };
    }
    let decoded = match decode_legacy(bytes, options.encoding) {
        Ok(text) => text,
        Err(_) => {
            return BibTexSource {
                entries: Vec::new(),
                preambles: Vec::new(),
                macros: month_macros(),
                diagnostics: vec![diagnostic(
                    BibTexDiagnosticKind::Encoding,
                    0,
                    "datasource cannot be decoded with the selected encoding",
                )],
            };
        }
    };
    let decoded = TexRecoder::new(options.decode, RecodeSet::Null).decode(&decoded);
    Parser::new(&decoded, options.limits).parse()
}

struct Parser<'a> {
    input: &'a str,
    at: usize,
    limits: BibTexLimits,
    work: usize,
    source: BibTexSource,
    keys: BTreeMap<String, String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, limits: BibTexLimits) -> Self {
        Self {
            input,
            at: 0,
            limits,
            work: 0,
            source: BibTexSource {
                entries: Vec::new(),
                preambles: Vec::new(),
                macros: month_macros(),
                diagnostics: Vec::new(),
            },
            keys: BTreeMap::new(),
        }
    }

    fn parse(mut self) -> BibTexSource {
        while self.at < self.input.len() && self.work < self.limits.max_work {
            self.work += 1;
            match self.rest().find('@') {
                Some(relative) => {
                    self.work = self.work.saturating_add(relative);
                    self.at += relative + 1;
                }
                None => {
                    self.work = self.work.saturating_add(self.rest().len());
                    break;
                }
            }
            let start = self.at - 1;
            self.space();
            let kind = self.identifier().to_ascii_lowercase();
            self.space();
            let Some(open) = self.peek() else { break };
            if open != b'{' && open != b'(' {
                self.error(
                    BibTexDiagnosticKind::Syntax,
                    start,
                    "expected `{` or `(` after entry type",
                );
                continue;
            }
            self.at += 1;
            let close = if open == b'{' { b'}' } else { b')' };
            match kind.as_str() {
                "comment" => self.skip_balanced(open, close),
                "preamble" => self.parse_preamble(close, start),
                "string" => self.parse_string(close, start),
                "" => self.recover(close),
                _ => self.parse_entry(kind, close, start),
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

    fn parse_preamble(&mut self, close: u8, start: usize) {
        match self.value(close, 0) {
            Some(value) => {
                self.source.preambles.push(BibTexPreamble(value));
                self.finish_record(close, start);
            }
            None => self.recover(close),
        }
    }

    fn parse_string(&mut self, close: u8, start: usize) {
        self.space();
        let name = self.identifier().to_ascii_lowercase();
        self.space();
        if name.is_empty() || !self.eat(b'=') {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "malformed string macro",
            );
            self.recover(close);
            return;
        }
        if self.source.macros.len() >= self.limits.max_macros {
            self.error(BibTexDiagnosticKind::Limit, start, "macro limit exceeded");
            self.recover(close);
            return;
        }
        if let Some(value) = self.value(close, 0) {
            self.source.macros.insert(name, value);
            self.finish_record(close, start);
        } else {
            self.recover(close);
        }
    }

    fn parse_entry(&mut self, entry_type: String, close: u8, start: usize) {
        if self.source.entries.len() >= self.limits.max_entries {
            self.error(BibTexDiagnosticKind::Limit, start, "entry limit exceeded");
            self.recover(close);
            return;
        }
        self.space();
        let key_start = self.at;
        while let Some(byte) = self.peek() {
            if byte == b',' || byte == close {
                break;
            }
            self.at += 1;
        }
        let key = self.input[key_start..self.at].trim().to_owned();
        if key.is_empty() || !self.eat(b',') {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "entry has no key or field separator",
            );
            self.recover(close);
            return;
        }
        let folded = key.to_ascii_lowercase();
        if let Some(previous) = self.keys.get(&folded) {
            let kind = if previous == &key {
                BibTexDiagnosticKind::DuplicateEntry
            } else {
                BibTexDiagnosticKind::CaseCollision
            };
            self.error(
                kind,
                key_start,
                format!("entry key `{key}` collides with earlier `{previous}`"),
            );
            self.recover(close);
            return;
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
            let name = self.identifier().to_ascii_lowercase();
            self.space();
            if name.is_empty() || !self.eat(b'=') {
                self.error(
                    BibTexDiagnosticKind::Syntax,
                    field_start,
                    "malformed field assignment",
                );
                self.recover(close);
                return;
            }
            if fields.len() >= self.limits.max_fields_per_entry {
                self.error(
                    BibTexDiagnosticKind::Limit,
                    field_start,
                    "field limit exceeded",
                );
                self.recover(close);
                return;
            }
            let Some(value) = self.value(close, 0) else {
                self.recover(close);
                return;
            };
            if fields.iter().any(|field: &BibTexField| field.name == name) {
                self.error(
                    BibTexDiagnosticKind::DuplicateField,
                    field_start,
                    format!("duplicate field `{name}` in entry `{key}`"),
                );
            } else {
                let names = is_name_field(&name).then(|| split_names(&value));
                fields.push(BibTexField { name, value, names });
            }
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
                self.recover(close);
                return;
            }
        }
        add_date_parts(&mut fields);
        self.keys.insert(folded, key.clone());
        self.source.entries.push(BibTexEntry {
            key,
            entry_type,
            fields,
        });
    }

    fn value(&mut self, close: u8, depth: usize) -> Option<String> {
        if depth > self.limits.max_nesting {
            self.error(
                BibTexDiagnosticKind::Limit,
                self.at,
                "value nesting limit exceeded",
            );
            return None;
        }
        let mut value = String::new();
        loop {
            self.space();
            let piece = match self.peek()? {
                b'{' => self.braced(depth + 1)?,
                b'"' => self.quoted(depth + 1)?,
                byte if byte.is_ascii_digit() => self.number(),
                _ => {
                    let offset = self.at;
                    let name = self.identifier().to_ascii_lowercase();
                    if name.is_empty() {
                        self.error(BibTexDiagnosticKind::Syntax, offset, "expected value");
                        return None;
                    }
                    match self.source.macros.get(&name) {
                        Some(value) => value.clone(),
                        None => {
                            self.error(
                                BibTexDiagnosticKind::UndefinedMacro,
                                offset,
                                format!("undefined string macro `{name}`"),
                            );
                            name
                        }
                    }
                }
            };
            if value.len().saturating_add(piece.len()) > self.limits.max_value_bytes {
                self.error(
                    BibTexDiagnosticKind::Limit,
                    self.at,
                    "value byte limit exceeded",
                );
                return None;
            }
            value.push_str(&piece);
            self.space();
            if !self.eat(b'#') {
                break;
            }
        }
        if self.peek() == Some(close) || self.peek() == Some(b',') {
            Some(value)
        } else {
            self.error(
                BibTexDiagnosticKind::Syntax,
                self.at,
                "unexpected token after value",
            );
            None
        }
    }

    fn braced(&mut self, depth: usize) -> Option<String> {
        self.at += 1;
        let start = self.at;
        let mut nesting = 1usize;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
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
                    return Some(self.input[start..self.at - 1].to_owned());
                }
            }
        }
        self.error(
            BibTexDiagnosticKind::Syntax,
            start,
            "unterminated braced value",
        );
        None
    }

    fn quoted(&mut self, depth: usize) -> Option<String> {
        self.at += 1;
        let start = self.at;
        let mut braces = 0usize;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
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
                return Some(self.input[start..self.at - 1].to_owned());
            }
        }
        self.error(
            BibTexDiagnosticKind::Syntax,
            start,
            "unterminated quoted value",
        );
        None
    }

    fn number(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        self.input[start..self.at].to_owned()
    }

    fn identifier(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(is_identifier_byte) {
            self.at += 1;
        }
        self.input[start..self.at].to_owned()
    }

    fn finish_record(&mut self, close: u8, start: usize) {
        self.space();
        if self.eat(b',') {
            self.space();
        }
        if !self.eat(close) {
            self.error(
                BibTexDiagnosticKind::Syntax,
                start,
                "record has trailing content",
            );
            self.recover(close);
        }
    }

    fn recover(&mut self, close: u8) {
        let mut braces = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
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
            if braces == 0 && (byte == close || byte == b'@') {
                if byte == b'@' {
                    self.at -= 1;
                }
                break;
            }
            if self.work >= self.limits.max_work {
                break;
            }
        }
    }

    fn skip_balanced(&mut self, open: u8, close: u8) {
        let mut nesting = 1usize;
        while let Some(byte) = self.peek() {
            self.at += 1;
            if byte == open {
                nesting += 1;
            }
            if byte == close {
                nesting -= 1;
                if nesting == 0 {
                    break;
                }
            }
        }
    }

    fn error(&mut self, kind: BibTexDiagnosticKind, offset: usize, message: impl Into<String>) {
        if self.source.diagnostics.len() < self.limits.max_diagnostics {
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
    fn rest(&self) -> &str {
        &self.input[self.at..]
    }
}

fn month_macros() -> BTreeMap<String, String> {
    [
        ("jan", "1"),
        ("feb", "2"),
        ("mar", "3"),
        ("apr", "4"),
        ("may", "5"),
        ("jun", "6"),
        ("jul", "7"),
        ("aug", "8"),
        ("sep", "9"),
        ("oct", "10"),
        ("nov", "11"),
        ("dec", "12"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

fn is_identifier_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace()
        && !matches!(
            byte,
            b'=' | b',' | b'{' | b'}' | b'(' | b')' | b'"' | b'#' | b'@'
        )
}

fn is_name_field(name: &str) -> bool {
    matches!(
        name,
        "author"
            | "editor"
            | "translator"
            | "commentator"
            | "annotator"
            | "introduction"
            | "foreword"
            | "afterword"
            | "bookauthor"
            | "holder"
            | "namea"
            | "nameb"
            | "namec"
    )
}

fn split_names(value: &str) -> Vec<RawName> {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut at = 0usize;
    let mut braces = 0usize;
    let mut names = Vec::new();
    while at < bytes.len() {
        match bytes[at] {
            b'{' => braces += 1,
            b'}' if braces > 0 => braces -= 1,
            b'a' | b'A'
                if braces == 0
                    && value[at..]
                        .get(..3)
                        .is_some_and(|part| part.eq_ignore_ascii_case("and")) =>
            {
                let before = at == 0 || bytes[at - 1].is_ascii_whitespace();
                let after = at + 3 == bytes.len() || bytes[at + 3].is_ascii_whitespace();
                if before && after {
                    let name = value[start..at].trim();
                    if !name.is_empty() {
                        names.push(RawName {
                            value: name.to_owned(),
                        });
                    }
                    at += 3;
                    start = at;
                    continue;
                }
            }
            _ => {}
        }
        at += 1;
    }
    let name = value[start..].trim();
    if !name.is_empty() {
        names.push(RawName {
            value: name.to_owned(),
        });
    }
    names
}

fn add_date_parts(fields: &mut Vec<BibTexField>) {
    let Some(date) = fields
        .iter()
        .find(|field| field.name == "date")
        .map(|field| field.value.clone())
    else {
        return;
    };
    let mut parts = date.split('-');
    let year = parts.next().filter(|value| value.len() == 4);
    let month = parts
        .next()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| (1..=12).contains(value));
    if let Some(year) = year
        && !fields.iter().any(|field| field.name == "year")
    {
        fields.push(BibTexField {
            name: "year".into(),
            value: year.into(),
            names: None,
        });
    }
    if let Some(month) = month
        && !fields.iter().any(|field| field.name == "month")
    {
        fields.push(BibTexField {
            name: "month".into(),
            value: month.to_string(),
            names: None,
        });
    }
}

fn diagnostic(
    kind: BibTexDiagnosticKind,
    offset: usize,
    message: impl Into<String>,
) -> BibTexDiagnostic {
    BibTexDiagnostic {
        kind,
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
