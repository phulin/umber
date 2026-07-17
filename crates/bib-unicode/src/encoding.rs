use encoding_rs::{Encoding, MACINTOSH, UTF_8, WINDOWS_1250, WINDOWS_1252};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyEncoding {
    Utf8,
    Latin1,
    Latin2,
    Latin3,
    MacRoman,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodingError {
    UnknownLabel,
    MalformedInput,
    UnmappableCharacter,
}

impl LegacyEncoding {
    pub fn for_label(label: &str) -> Result<Self, EncodingError> {
        match label.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
            "utf8" => Ok(Self::Utf8),
            "latin1" | "iso88591" => Ok(Self::Latin1),
            "latin2" | "iso88592" => Ok(Self::Latin2),
            "latin3" | "iso88593" => Ok(Self::Latin3),
            "macroman" | "applemac" | "applemacce" => Ok(Self::MacRoman),
            _ => Err(EncodingError::UnknownLabel),
        }
    }

    fn codec(self) -> &'static Encoding {
        match self {
            Self::Utf8 => UTF_8,
            Self::Latin1 => WINDOWS_1252,
            Self::Latin2 => WINDOWS_1250,
            Self::Latin3 => WINDOWS_1250,
            Self::MacRoman => MACINTOSH,
        }
    }
}

pub fn decode_legacy(bytes: &[u8], encoding: LegacyEncoding) -> Result<String, EncodingError> {
    let (text, had_errors) = encoding.codec().decode_without_bom_handling(bytes);
    if had_errors {
        Err(EncodingError::MalformedInput)
    } else {
        Ok(text.into_owned())
    }
}

pub fn encode_legacy(value: &str, encoding: LegacyEncoding) -> Result<Vec<u8>, EncodingError> {
    let (bytes, _, had_errors) = encoding.codec().encode(value);
    if had_errors {
        Err(EncodingError::UnmappableCharacter)
    } else {
        Ok(bytes.into_owned())
    }
}
