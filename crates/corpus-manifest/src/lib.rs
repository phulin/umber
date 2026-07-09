#![allow(clippy::disallowed_methods)] // Host-side manifest parser reads repository files.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Manifest {
    pub doc: Vec<Document>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Document {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
    pub redistributable: bool,
    pub expected_ref_dvi_sha256: String,
    pub notes: String,
}

#[derive(Debug)]
pub struct ManifestError {
    path: Option<PathBuf>,
    line: Option<usize>,
    message: String,
}

impl ManifestError {
    fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            path: None,
            line,
            message: message.into(),
        }
    }

    fn with_path(mut self, path: &Path) -> Self {
        self.path = Some(path.to_path_buf());
        self
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, self.line) {
            (Some(path), Some(line)) => {
                write!(f, "{}:{}: {}", path.display(), line, self.message)
            }
            (Some(path), None) => write!(f, "{}: {}", path.display(), self.message),
            (None, Some(line)) => write!(f, "line {line}: {}", self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

impl Error for ManifestError {}

pub fn parse_manifest_file(path: &Path) -> Result<Manifest, ManifestError> {
    let text = fs::read_to_string(path).map_err(|error| {
        ManifestError::new(None, format!("failed to read manifest: {error}")).with_path(path)
    })?;
    parse_manifest(&text).map_err(|error| error.with_path(path))
}

pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let mut builder = ManifestBuilder::default();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let (key, value) = split_key_value(trimmed, line_number)?;
        builder.push_field(key, value, line_number)?;
    }
    builder.finish()
}

fn split_key_value(line: &str, line_number: usize) -> Result<(&str, &str), ManifestError> {
    let Some((key_end, _)) = line.char_indices().find(|(_, ch)| ch.is_ascii_whitespace()) else {
        return Err(ManifestError::new(
            Some(line_number),
            "manifest line must contain a key and value",
        ));
    };
    let key = &line[..key_end];
    let value = line[key_end..].trim_start();
    if value.is_empty() {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{key} has an empty value"),
        ));
    }
    Ok((key, value))
}

#[derive(Default)]
struct ManifestBuilder {
    documents: Vec<Document>,
    current: Option<DocumentBuilder>,
}

impl ManifestBuilder {
    fn push_field(
        &mut self,
        key: &str,
        value: &str,
        line_number: usize,
    ) -> Result<(), ManifestError> {
        if key == "doc" {
            self.finish_current()?;
            self.current = Some(DocumentBuilder::new(value.to_string(), line_number));
            return Ok(());
        }

        let Some(current) = self.current.as_mut() else {
            return Err(ManifestError::new(
                Some(line_number),
                "manifest entries must begin with a doc line",
            ));
        };
        current.set_field(key, value, line_number)
    }

    fn finish(mut self) -> Result<Manifest, ManifestError> {
        self.finish_current()?;
        if self.documents.is_empty() {
            return Err(ManifestError::new(
                None,
                "manifest does not contain any doc entries",
            ));
        }
        Ok(Manifest {
            doc: self.documents,
        })
    }

    fn finish_current(&mut self) -> Result<(), ManifestError> {
        if let Some(current) = self.current.take() {
            self.documents.push(current.finish()?);
        }
        Ok(())
    }
}

struct DocumentBuilder {
    name: String,
    start_line: usize,
    seen: HashSet<&'static str>,
    url: Option<String>,
    sha256: Option<String>,
    license: Option<String>,
    redistributable: Option<bool>,
    expected_ref_dvi_sha256: Option<String>,
    notes: Option<String>,
}

impl DocumentBuilder {
    fn new(name: String, start_line: usize) -> Self {
        Self {
            name,
            start_line,
            seen: HashSet::new(),
            url: None,
            sha256: None,
            license: None,
            redistributable: None,
            expected_ref_dvi_sha256: None,
            notes: None,
        }
    }

    fn set_field(
        &mut self,
        key: &str,
        value: &str,
        line_number: usize,
    ) -> Result<(), ManifestError> {
        let canonical = match key {
            "url" => "url",
            "sha256" => "sha256",
            "license" => "license",
            "redistributable" => "redistributable",
            "expected_ref_dvi_sha256" => "expected_ref_dvi_sha256",
            "notes" => "notes",
            _ => {
                return Err(ManifestError::new(
                    Some(line_number),
                    format!("unknown manifest field: {key}"),
                ));
            }
        };
        if !self.seen.insert(canonical) {
            return Err(ManifestError::new(
                Some(line_number),
                format!("duplicate manifest field: {key}"),
            ));
        }

        match canonical {
            "url" => self.url = Some(value.to_string()),
            "sha256" => self.sha256 = Some(value.to_string()),
            "license" => self.license = Some(value.to_string()),
            "redistributable" => {
                self.redistributable = Some(parse_bool(value, line_number)?);
            }
            "expected_ref_dvi_sha256" => {
                self.expected_ref_dvi_sha256 = Some(value.to_string());
            }
            "notes" => self.notes = Some(value.to_string()),
            _ => unreachable!("validated canonical field"),
        }
        Ok(())
    }

    fn finish(self) -> Result<Document, ManifestError> {
        let doc = Document {
            name: self.name,
            url: required(self.url, "url", self.start_line)?,
            sha256: required(self.sha256, "sha256", self.start_line)?,
            license: required(self.license, "license", self.start_line)?,
            redistributable: required(self.redistributable, "redistributable", self.start_line)?,
            expected_ref_dvi_sha256: required(
                self.expected_ref_dvi_sha256,
                "expected_ref_dvi_sha256",
                self.start_line,
            )?,
            notes: required(self.notes, "notes", self.start_line)?,
        };
        validate_document(&doc, self.start_line)?;
        Ok(doc)
    }
}

fn required<T>(value: Option<T>, field: &str, line_number: usize) -> Result<T, ManifestError> {
    value.ok_or_else(|| {
        ManifestError::new(
            Some(line_number),
            format!("doc entry is missing required field: {field}"),
        )
    })
}

fn parse_bool(value: &str, line_number: usize) -> Result<bool, ManifestError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ManifestError::new(
            Some(line_number),
            format!("redistributable must be true or false, got {value}"),
        )),
    }
}

fn validate_document(doc: &Document, line_number: usize) -> Result<(), ManifestError> {
    if !safe_file_name(&doc.name) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("invalid corpus document name: {}", doc.name),
        ));
    }
    if !(doc.url.starts_with("https://") || doc.url.starts_with("http://")) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{} has unsupported URL scheme: {}", doc.name, doc.url),
        ));
    }
    if !is_sha256(&doc.sha256) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{} has invalid sha256: {}", doc.name, doc.sha256),
        ));
    }
    if !is_sha256(&doc.expected_ref_dvi_sha256) {
        return Err(ManifestError::new(
            Some(line_number),
            format!(
                "{} has invalid expected_ref_dvi_sha256: {}",
                doc.name, doc.expected_ref_dvi_sha256
            ),
        ));
    }
    if doc.license.trim().is_empty() {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{} has an empty license field", doc.name),
        ));
    }
    if doc.notes.trim().is_empty() {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{} has empty licensing notes", doc.name),
        ));
    }
    if !doc.redistributable && doc.license != "no-redistribution" {
        return Err(ManifestError::new(
            Some(line_number),
            format!(
                "{} is marked non-redistributable but license is {}; use no-redistribution",
                doc.name, doc.license
            ),
        ));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('/')
        && !value.contains('\\')
        && value != "."
        && value != ".."
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_multiple_documents() {
        let manifest = parse_manifest(&format!(
            r#"
# corpus

doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license Knuth-CTAN
redistributable true
expected_ref_dvi_sha256 {HASH}
notes fixture notes may contain spaces

doc gentle.tex
url http://example.com/gentle.tex
sha256 {HASH}
license MIT
redistributable true
expected_ref_dvi_sha256 {HASH}
notes another fixture
"#
        ))
        .expect("manifest should parse");

        assert_eq!(manifest.doc.len(), 2);
        assert_eq!(manifest.doc[0].name, "story.tex");
        assert_eq!(manifest.doc[0].notes, "fixture notes may contain spaces");
        assert_eq!(manifest.doc[1].url, "http://example.com/gentle.tex");
    }

    #[test]
    fn parses_committed_manifest() {
        let manifest = parse_manifest(include_str!("../../../tests/corpus-manifest.txt"))
            .expect("committed manifest should parse");

        assert!(!manifest.doc.is_empty());
    }

    #[test]
    fn rejects_unknown_field() {
        let error = parse_manifest(&format!(
            r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
bogus value
license MIT
redistributable true
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("unknown field should fail");

        assert!(error.to_string().contains("unknown manifest field: bogus"));
    }

    #[test]
    fn rejects_duplicate_field() {
        let error = parse_manifest(&format!(
            r#"
doc story.tex
url https://example.com/story.tex
url https://example.com/other.tex
sha256 {HASH}
license MIT
redistributable true
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("duplicate field should fail");

        assert!(error.to_string().contains("duplicate manifest field: url"));
    }

    #[test]
    fn rejects_missing_field() {
        let error = parse_manifest(&format!(
            r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("missing field should fail");

        assert!(
            error
                .to_string()
                .contains("missing required field: redistributable")
        );
    }

    #[test]
    fn rejects_bad_hash() {
        let error = parse_manifest(&format!(
            r#"
doc story.tex
url https://example.com/story.tex
sha256 nope
license MIT
redistributable true
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("bad hash should fail");

        assert!(error.to_string().contains("has invalid sha256"));
    }

    #[test]
    fn rejects_path_traversal_document_name() {
        let error = parse_manifest(&format!(
            r#"
doc ../story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
redistributable true
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("unsafe file name should fail");

        assert!(
            error
                .to_string()
                .contains("invalid corpus document name: ../story.tex")
        );
    }

    #[test]
    fn rejects_bad_bool() {
        let error = parse_manifest(&format!(
            r#"
doc story.tex
url https://example.com/story.tex
sha256 {HASH}
license MIT
redistributable yes
expected_ref_dvi_sha256 {HASH}
notes fixture
"#
        ))
        .expect_err("bad bool should fail");

        assert!(
            error
                .to_string()
                .contains("redistributable must be true or false")
        );
    }
}
