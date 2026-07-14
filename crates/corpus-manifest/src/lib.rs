#![allow(clippy::disallowed_methods)] // Host-side manifest parser reads repository files.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Manifest {
    pub support: Vec<SupportFile>,
    pub doc: Vec<Document>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SupportFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
    pub redistributable: bool,
    pub notes: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Document {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
    pub redistributable: bool,
    pub format_source: String,
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
    support: Vec<SupportFile>,
    documents: Vec<Document>,
    current: Option<EntryBuilder>,
}

enum EntryBuilder {
    Support(SupportBuilder),
    Document(DocumentBuilder),
}

impl ManifestBuilder {
    fn push_field(
        &mut self,
        key: &str,
        value: &str,
        line_number: usize,
    ) -> Result<(), ManifestError> {
        if matches!(key, "support" | "doc") {
            self.finish_current()?;
            self.current = Some(if key == "support" {
                EntryBuilder::Support(SupportBuilder::new(value.to_string(), line_number))
            } else {
                EntryBuilder::Document(DocumentBuilder::new(value.to_string(), line_number))
            });
            return Ok(());
        }

        let Some(current) = self.current.as_mut() else {
            return Err(ManifestError::new(
                Some(line_number),
                "manifest entries must begin with a support or doc line",
            ));
        };
        match current {
            EntryBuilder::Support(current) => current.set_field(key, value, line_number),
            EntryBuilder::Document(current) => current.set_field(key, value, line_number),
        }
    }

    fn finish(mut self) -> Result<Manifest, ManifestError> {
        self.finish_current()?;
        if self.documents.is_empty() {
            return Err(ManifestError::new(
                None,
                "manifest does not contain any doc entries",
            ));
        }
        let mut names = BTreeSet::new();
        for file in &self.support {
            if !names.insert(file.name.as_str()) {
                return Err(ManifestError::new(
                    None,
                    format!("duplicate corpus file name: {}", file.name),
                ));
            }
        }
        for doc in &self.documents {
            if !names.insert(doc.name.as_str()) {
                return Err(ManifestError::new(
                    None,
                    format!("duplicate corpus file name: {}", doc.name),
                ));
            }
            if !self
                .support
                .iter()
                .any(|file| file.name == doc.format_source)
            {
                return Err(ManifestError::new(
                    None,
                    format!(
                        "{} references missing format_source {}",
                        doc.name, doc.format_source
                    ),
                ));
            }
        }
        Ok(Manifest {
            support: self.support,
            doc: self.documents,
        })
    }

    fn finish_current(&mut self) -> Result<(), ManifestError> {
        if let Some(current) = self.current.take() {
            match current {
                EntryBuilder::Support(current) => self.support.push(current.finish()?),
                EntryBuilder::Document(current) => self.documents.push(current.finish()?),
            }
        }
        Ok(())
    }
}

struct SupportBuilder {
    name: String,
    start_line: usize,
    seen: BTreeSet<&'static str>,
    url: Option<String>,
    sha256: Option<String>,
    license: Option<String>,
    redistributable: Option<bool>,
    notes: Option<String>,
}

impl SupportBuilder {
    fn new(name: String, start_line: usize) -> Self {
        Self {
            name,
            start_line,
            seen: BTreeSet::new(),
            url: None,
            sha256: None,
            license: None,
            redistributable: None,
            notes: None,
        }
    }

    fn set_field(
        &mut self,
        key: &str,
        value: &str,
        line_number: usize,
    ) -> Result<(), ManifestError> {
        let canonical = common_field(key, line_number)?;
        if !self.seen.insert(canonical) {
            return Err(ManifestError::new(
                Some(line_number),
                format!("duplicate manifest field: {key}"),
            ));
        }
        set_common_field(
            canonical,
            value,
            line_number,
            &mut self.url,
            &mut self.sha256,
            &mut self.license,
            &mut self.redistributable,
            &mut self.notes,
        )
    }

    fn finish(self) -> Result<SupportFile, ManifestError> {
        let file = SupportFile {
            name: self.name,
            url: required(self.url, "url", self.start_line)?,
            sha256: required(self.sha256, "sha256", self.start_line)?,
            license: required(self.license, "license", self.start_line)?,
            redistributable: required(self.redistributable, "redistributable", self.start_line)?,
            notes: required(self.notes, "notes", self.start_line)?,
        };
        validate_file(
            &file.name,
            &file.url,
            &file.sha256,
            &file.license,
            file.redistributable,
            &file.notes,
            self.start_line,
        )?;
        Ok(file)
    }
}

struct DocumentBuilder {
    name: String,
    start_line: usize,
    seen: BTreeSet<&'static str>,
    url: Option<String>,
    sha256: Option<String>,
    license: Option<String>,
    redistributable: Option<bool>,
    format_source: Option<String>,
    expected_ref_dvi_sha256: Option<String>,
    notes: Option<String>,
}

impl DocumentBuilder {
    fn new(name: String, start_line: usize) -> Self {
        Self {
            name,
            start_line,
            seen: BTreeSet::new(),
            url: None,
            sha256: None,
            license: None,
            redistributable: None,
            format_source: None,
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
            "format_source" => "format_source",
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
            "format_source" => self.format_source = Some(value.to_string()),
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
            format_source: required(self.format_source, "format_source", self.start_line)?,
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

fn common_field(key: &str, line_number: usize) -> Result<&'static str, ManifestError> {
    match key {
        "url" => Ok("url"),
        "sha256" => Ok("sha256"),
        "license" => Ok("license"),
        "redistributable" => Ok("redistributable"),
        "notes" => Ok("notes"),
        _ => Err(ManifestError::new(
            Some(line_number),
            format!("unknown manifest field: {key}"),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn set_common_field(
    key: &str,
    value: &str,
    line_number: usize,
    url: &mut Option<String>,
    sha256: &mut Option<String>,
    license: &mut Option<String>,
    redistributable: &mut Option<bool>,
    notes: &mut Option<String>,
) -> Result<(), ManifestError> {
    match key {
        "url" => *url = Some(value.to_string()),
        "sha256" => *sha256 = Some(value.to_string()),
        "license" => *license = Some(value.to_string()),
        "redistributable" => *redistributable = Some(parse_bool(value, line_number)?),
        "notes" => *notes = Some(value.to_string()),
        _ => unreachable!("validated common field"),
    }
    Ok(())
}

fn validate_document(doc: &Document, line_number: usize) -> Result<(), ManifestError> {
    validate_file(
        &doc.name,
        &doc.url,
        &doc.sha256,
        &doc.license,
        doc.redistributable,
        &doc.notes,
        line_number,
    )?;
    if !safe_file_name(&doc.format_source) {
        return Err(ManifestError::new(
            Some(line_number),
            format!(
                "{} has invalid format_source: {}",
                doc.name, doc.format_source
            ),
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
    Ok(())
}

fn validate_file(
    name: &str,
    url: &str,
    sha256: &str,
    license: &str,
    redistributable: bool,
    notes: &str,
    line_number: usize,
) -> Result<(), ManifestError> {
    if !safe_file_name(name) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("invalid corpus file name: {name}"),
        ));
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{name} has unsupported URL scheme: {url}"),
        ));
    }
    if !is_sha256(sha256) {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{name} has invalid sha256: {sha256}"),
        ));
    }
    if license.trim().is_empty() {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{name} has an empty license field"),
        ));
    }
    if notes.trim().is_empty() {
        return Err(ManifestError::new(
            Some(line_number),
            format!("{name} has empty licensing notes"),
        ));
    }
    if !redistributable && license != "no-redistribution" {
        return Err(ManifestError::new(
            Some(line_number),
            format!(
                "{name} is marked non-redistributable but license is {license}; use no-redistribution"
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
mod tests;
