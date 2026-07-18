//! Host-neutral parsing for pdfTeX/dvips font-map directives.
//!
//! The parser accepts bytes already acquired by a frontend. It deliberately
//! performs no path lookup: names in a map remain logical resource names until
//! a driver resolves them through its resource contract.

/// How a `\pdfmapfile` or `\pdfmapline` payload updates the live map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfFontMapDirective {
    /// The unprefixed pdfTeX operation.
    Default,
    /// Add entries while preserving unrelated entries (`+`).
    Add,
    /// Replace the configured map source (`=`).
    Replace,
    /// Remove the named source or entry (`-`).
    Remove,
}

impl PdfFontMapDirective {
    /// Splits the optional one-byte pdfTeX update prefix from a payload.
    #[must_use]
    pub fn split(payload: &[u8]) -> (Self, &[u8]) {
        match payload.first() {
            Some(b'+') => (Self::Add, &payload[1..]),
            Some(b'=') => (Self::Replace, &payload[1..]),
            Some(b'-') => (Self::Remove, &payload[1..]),
            _ => (Self::Default, payload),
        }
    }
}

/// A logical map-file request; bytes are acquired outside the engine.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontMapFile {
    pub directive: PdfFontMapDirective,
    pub logical_name: Vec<u8>,
}

/// Validated contents of one acquired dvips/pdfTeX map file.
///
/// Map files are parsed after a host supplies their bytes. Keeping this type
/// independent of path lookup lets native and browser frontends share the
/// same engine-state contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFontMap {
    entries: Vec<PdfFontMapEntry>,
}

impl PdfFontMap {
    pub fn parse(bytes: &[u8]) -> Result<Self, PdfFontMapError> {
        let mut entries = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            let line = trim_ascii(line);
            if line.is_empty() || line.starts_with(b"%") {
                continue;
            }
            let mut entry = PdfFontMapEntry::parse(line)?;
            // Prefixes are primitive update syntax, not part of a map-file
            // entry. The enclosing \pdfmapfile operation supplies the update
            // policy when these entries are projected into live state.
            entry.directive = PdfFontMapDirective::Default;
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[PdfFontMapEntry] {
        &self.entries
    }
}

impl PdfFontMapFile {
    pub fn parse(payload: &[u8]) -> Result<Self, PdfFontMapError> {
        let (directive, name) = PdfFontMapDirective::split(trim_ascii(payload));
        let logical_name = trim_ascii(name);
        if logical_name.is_empty() {
            return Err(PdfFontMapError::MissingMapFileName);
        }
        if logical_name.iter().any(|byte| byte.is_ascii_whitespace()) {
            return Err(PdfFontMapError::WhitespaceInMapFileName);
        }
        Ok(Self {
            directive,
            logical_name: logical_name.to_vec(),
        })
    }
}

/// Embedding policy encoded by `<` and `<<` download tokens.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfFontMapProgram {
    /// Reference a resident font; no program token was supplied.
    Resident,
    /// Embed a subset (`<font.pfb`).
    Subset,
    /// Embed the complete program (`<<font.pfb`).
    Full,
}

/// One parsed dvips/pdfTeX map entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontMapEntry {
    pub directive: PdfFontMapDirective,
    pub tex_name: Vec<u8>,
    pub postscript_name: Option<Vec<u8>>,
    pub special_instructions: Vec<Vec<u8>>,
    pub encoding_files: Vec<Vec<u8>>,
    pub header_files: Vec<Vec<u8>>,
    pub font_file: Option<Vec<u8>>,
    pub program: PdfFontMapProgram,
}

impl PdfFontMapEntry {
    /// Parses one `\pdfmapline` payload or one non-comment map-file line.
    pub fn parse(payload: &[u8]) -> Result<Self, PdfFontMapError> {
        let (directive, body) = PdfFontMapDirective::split(trim_ascii(payload));
        let tokens = coalesce_download_markers(lex_map_line(body)?)?;
        let Some(first) = tokens.first() else {
            return Err(PdfFontMapError::MissingTexFontName);
        };
        if first.quoted || first.bytes.starts_with(b"<") {
            return Err(PdfFontMapError::MissingTexFontName);
        }

        let mut postscript_name = None;
        let mut special_instructions = Vec::new();
        let mut encoding_files = Vec::new();
        let mut header_files = Vec::new();
        let mut font_file = None;
        let mut program = PdfFontMapProgram::Resident;

        for token in &tokens[1..] {
            if token.quoted {
                special_instructions.push(token.bytes.clone());
                continue;
            }
            if let Some(download) = token.bytes.strip_prefix(b"<<") {
                if is_header_name(download) {
                    header_files.push(download.to_vec());
                } else {
                    set_font_file(&mut font_file, download)?;
                    program = PdfFontMapProgram::Full;
                }
                continue;
            }
            if let Some(download) = token.bytes.strip_prefix(b"<[") {
                // dvips/pdfTeX map syntax uses `<[encoding.enc`; tolerate the
                // visually balanced spelling too because existing mapline
                // producers emit both forms.
                let encoding = download.strip_suffix(b"]").unwrap_or(download);
                set_encoding(&mut encoding_files, encoding)?;
                continue;
            }
            if let Some(download) = token.bytes.strip_prefix(b"<") {
                if download.is_empty() {
                    return Err(PdfFontMapError::MalformedDownloadToken(token.bytes.clone()));
                }
                if is_encoding_name(download) {
                    set_encoding(&mut encoding_files, download)?;
                } else if is_header_name(download) {
                    header_files.push(download.to_vec());
                } else {
                    set_font_file(&mut font_file, download)?;
                    program = PdfFontMapProgram::Subset;
                }
                continue;
            }
            if postscript_name.is_some() {
                return Err(PdfFontMapError::UnexpectedBareToken(token.bytes.clone()));
            }
            postscript_name = Some(token.bytes.clone());
        }

        Ok(Self {
            directive,
            tex_name: first.bytes.clone(),
            postscript_name,
            special_instructions,
            encoding_files,
            header_files,
            font_file,
            program,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFontMapError {
    MissingMapFileName,
    WhitespaceInMapFileName,
    MissingTexFontName,
    UnterminatedQuote,
    EmptyQuotedInstruction,
    MalformedDownloadToken(Vec<u8>),
    DuplicateFontFile,
    EmptyEncodingFile,
    UnexpectedBareToken(Vec<u8>),
}

impl std::fmt::Display for PdfFontMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid pdfTeX font map: {self:?}")
    }
}

impl std::error::Error for PdfFontMapError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MapToken {
    bytes: Vec<u8>,
    quoted: bool,
}

fn lex_map_line(bytes: &[u8]) -> Result<Vec<MapToken>, PdfFontMapError> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == bytes.len() || bytes[index] == b'%' {
            break;
        }
        if bytes[index] == b'"' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            if index == bytes.len() {
                return Err(PdfFontMapError::UnterminatedQuote);
            }
            if start == index {
                return Err(PdfFontMapError::EmptyQuotedInstruction);
            }
            tokens.push(MapToken {
                bytes: bytes[start..index].to_vec(),
                quoted: true,
            });
            index += 1;
        } else {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'%'
            {
                index += 1;
            }
            tokens.push(MapToken {
                bytes: bytes[start..index].to_vec(),
                quoted: false,
            });
            if index < bytes.len() && bytes[index] == b'%' {
                break;
            }
        }
    }
    Ok(tokens)
}

fn coalesce_download_markers(tokens: Vec<MapToken>) -> Result<Vec<MapToken>, PdfFontMapError> {
    let mut coalesced = Vec::with_capacity(tokens.len());
    let mut tokens = tokens.into_iter();
    while let Some(mut token) = tokens.next() {
        if !token.quoted && matches!(token.bytes.as_slice(), b"<" | b"<<" | b"<[") {
            let Some(next) = tokens.next() else {
                return Err(PdfFontMapError::MalformedDownloadToken(token.bytes));
            };
            if next.quoted || next.bytes.starts_with(b"<") {
                return Err(PdfFontMapError::MalformedDownloadToken(token.bytes));
            }
            token.bytes.extend_from_slice(&next.bytes);
        }
        coalesced.push(token);
    }
    Ok(coalesced)
}

fn set_font_file(slot: &mut Option<Vec<u8>>, name: &[u8]) -> Result<(), PdfFontMapError> {
    if name.is_empty() {
        return Err(PdfFontMapError::MalformedDownloadToken(Vec::new()));
    }
    if slot.replace(name.to_vec()).is_some() {
        return Err(PdfFontMapError::DuplicateFontFile);
    }
    Ok(())
}

fn set_encoding(encodings: &mut Vec<Vec<u8>>, name: &[u8]) -> Result<(), PdfFontMapError> {
    if name.is_empty() {
        return Err(PdfFontMapError::EmptyEncodingFile);
    }
    encodings.push(name.to_vec());
    Ok(())
}

fn is_encoding_name(name: &[u8]) -> bool {
    name.ends_with(b".enc")
}

fn is_header_name(name: &[u8]) -> bool {
    name.ends_with(b".t3")
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_pdftex_map_entry_without_resolving_resources() {
        let entry = PdfFontMapEntry::parse(
            br#"+ptmr8r Times-Roman "TeXBase1Encoding ReEncodeFont" <8r.enc <utmr8a.pfb"#,
        )
        .expect("valid map entry");
        assert_eq!(entry.directive, PdfFontMapDirective::Add);
        assert_eq!(entry.tex_name, b"ptmr8r");
        assert_eq!(
            entry.postscript_name.as_deref(),
            Some(b"Times-Roman".as_slice())
        );
        assert_eq!(
            entry.special_instructions,
            [b"TeXBase1Encoding ReEncodeFont"]
        );
        assert_eq!(entry.encoding_files, [b"8r.enc"]);
        assert_eq!(entry.font_file.as_deref(), Some(b"utmr8a.pfb".as_slice()));
        assert_eq!(entry.program, PdfFontMapProgram::Subset);
    }

    #[test]
    fn parses_full_embedding_and_bracketed_encodings() {
        let entry = PdfFontMapEntry::parse(b"foo FooPS <[foo.enc] <<foo.pfb % ignored")
            .expect("valid map entry");
        assert_eq!(entry.encoding_files, [b"foo.enc"]);
        assert_eq!(entry.font_file.as_deref(), Some(b"foo.pfb".as_slice()));
        assert_eq!(entry.program, PdfFontMapProgram::Full);
    }

    #[test]
    fn parses_canonical_unclosed_encoding_download_marker() {
        let entry = PdfFontMapEntry::parse(b"foo FooPS <[foo.enc <foo.pfb")
            .expect("canonical dvips encoding marker parses");
        assert_eq!(entry.encoding_files, [b"foo.enc"]);
        assert_eq!(entry.font_file.as_deref(), Some(b"foo.pfb".as_slice()));
    }

    #[test]
    fn parses_spaced_markers_and_type3_headers_before_font_programs() {
        let entry = PdfFontMapEntry::parse(b"foo FooPS < foo.enc <<Foo-Menukad.t3 < Foo.pfb")
            .expect("generated map entry parses");
        assert_eq!(entry.encoding_files, [b"foo.enc"]);
        assert_eq!(entry.header_files, [b"Foo-Menukad.t3"]);
        assert_eq!(entry.font_file.as_deref(), Some(b"Foo.pfb".as_slice()));
    }

    #[test]
    fn map_file_request_is_a_logical_name_and_update_directive() {
        assert_eq!(
            PdfFontMapFile::parse(b" =pdftex.map ").expect("valid map file directive"),
            PdfFontMapFile {
                directive: PdfFontMapDirective::Replace,
                logical_name: b"pdftex.map".to_vec(),
            }
        );
        assert_eq!(
            PdfFontMapFile::parse(b"path with spaces.map"),
            Err(PdfFontMapError::WhitespaceInMapFileName)
        );
    }

    #[test]
    fn parses_map_file_comments_and_crlf_without_host_lookup() {
        let map = PdfFontMap::parse(
            b"% generated map\r\ncmr10 CMR10 <cmr10.pfb\r\n\r\ncmtt10 CMTT10 <cmtt10.pfb\n",
        )
        .expect("valid map file");
        assert_eq!(map.entries().len(), 2);
        assert_eq!(map.entries()[0].tex_name, b"cmr10");
        assert_eq!(
            map.entries()[1].font_file.as_deref(),
            Some(b"cmtt10.pfb".as_slice())
        );
    }

    #[test]
    fn rejects_ambiguous_or_truncated_entries() {
        assert_eq!(
            PdfFontMapEntry::parse(br#"foo FooPS "unterminated"#),
            Err(PdfFontMapError::UnterminatedQuote)
        );
        assert_eq!(
            PdfFontMapEntry::parse(b"foo FooPS <a.pfb <b.pfb"),
            Err(PdfFontMapError::DuplicateFontFile)
        );
    }
}
