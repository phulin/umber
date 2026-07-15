//! Host-neutral parsing of PostScript encoding-vector resources.

/// A validated 256-entry PostScript glyph-name encoding.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfEncoding {
    name: Vec<u8>,
    glyph_names: Vec<Vec<u8>>,
}

impl PdfEncoding {
    pub fn parse(bytes: &[u8]) -> Result<Self, PdfEncodingError> {
        let tokens = tokens(bytes);
        let open = tokens
            .iter()
            .position(|token| token.as_slice() == b"[")
            .ok_or(PdfEncodingError::MissingVector)?;
        let close = tokens[open + 1..]
            .iter()
            .position(|token| token.as_slice() == b"]")
            .map(|index| open + 1 + index)
            .ok_or(PdfEncodingError::UnterminatedVector)?;
        let name = tokens[..open]
            .iter()
            .rev()
            .find_map(|token| token.strip_prefix(b"/"))
            .filter(|name| !name.is_empty())
            .ok_or(PdfEncodingError::MissingName)?
            .to_vec();
        let mut glyph_names = Vec::with_capacity(256);
        for token in &tokens[open + 1..close] {
            let name = token
                .strip_prefix(b"/")
                .filter(|name| !name.is_empty())
                .ok_or_else(|| PdfEncodingError::InvalidGlyphName(token.clone()))?;
            if glyph_names.len() == 256 {
                return Err(PdfEncodingError::TooManyGlyphs);
            }
            glyph_names.push(name.to_vec());
        }
        if glyph_names.len() != 256 {
            return Err(PdfEncodingError::WrongGlyphCount(glyph_names.len()));
        }
        Ok(Self { name, glyph_names })
    }

    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    #[must_use]
    pub fn glyph_names(&self) -> &[Vec<u8>] {
        &self.glyph_names
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfEncodingError {
    MissingName,
    MissingVector,
    UnterminatedVector,
    InvalidGlyphName(Vec<u8>),
    TooManyGlyphs,
    WrongGlyphCount(usize),
}

impl std::fmt::Display for PdfEncodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid PostScript encoding: {self:?}")
    }
}

impl std::error::Error for PdfEncodingError {}

fn tokens(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n' || *byte == b'\r') {
        let line = line.split(|byte| *byte == b'%').next().unwrap_or_default();
        let mut start = None;
        for (index, byte) in line.iter().copied().chain([b' ']).enumerate() {
            if matches!(byte, b'[' | b']') {
                if let Some(begin) = start.take() {
                    result.push(line[begin..index].to_vec());
                }
                result.push(vec![byte]);
            } else if byte.is_ascii_whitespace() {
                if let Some(begin) = start.take() {
                    result.push(line[begin..index].to_vec());
                }
            } else if start.is_none() {
                start = Some(index);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_vector_and_comments() {
        let mut source = b"/FixtureEncoding [\n% comment\n".to_vec();
        for index in 0..256 {
            source.extend_from_slice(format!("/g{index} ").as_bytes());
        }
        source.extend_from_slice(b"] def\n");
        let encoding = PdfEncoding::parse(&source).expect("valid encoding");
        assert_eq!(encoding.name(), b"FixtureEncoding");
        assert_eq!(encoding.glyph_names()[0], b"g0");
        assert_eq!(encoding.glyph_names()[255], b"g255");
    }

    #[test]
    fn rejects_short_vectors() {
        assert_eq!(
            PdfEncoding::parse(b"/Short [/A] def"),
            Err(PdfEncodingError::WrongGlyphCount(1))
        );
    }
}
