//! Immutable character values and command-profile identity.

use std::fmt;

const UNICODE_SCALAR_TAG: u32 = 1 << 31;
const PROFILE_ENCODING_VERSION: u8 = 1;
const PROFILE_FINGERPRINT_DOMAIN: &[u8] = b"umber.tex-command.profile.v1\0";

/// A semantic input character without conflating bytes and Unicode scalars.
///
/// The private representation tags Unicode scalars, so byte `b'A'` and scalar
/// `U+0041` remain distinct values. Callers must explicitly select the domain
/// when constructing or extracting a code.
#[repr(transparent)]
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CharacterCode(u32);

/// A rejected [`CharacterCode`] construction or domain conversion.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CharacterCodeError {
    /// The supplied integer is not a Unicode scalar value.
    InvalidUnicodeScalar(u32),
    /// A Unicode scalar was requested from a byte-domain value.
    ExpectedUnicodeScalar,
    /// A byte was requested from a Unicode-scalar-domain value.
    ExpectedByte,
    /// Stable bytes contained a value outside both canonical encodings.
    InvalidEncoding(u32),
}

impl CharacterCode {
    /// Stable encoded width used by [`Self::to_stable_bytes`].
    pub const STABLE_ENCODED_LEN: usize = 4;

    /// Constructs an exact eight-bit character code.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        Self(byte as u32)
    }

    /// Constructs a Unicode-domain code after scalar validation.
    pub fn from_unicode_scalar(scalar: u32) -> Result<Self, CharacterCodeError> {
        char::from_u32(scalar)
            .map(|_| Self(UNICODE_SCALAR_TAG | scalar))
            .ok_or(CharacterCodeError::InvalidUnicodeScalar(scalar))
    }

    /// Returns whether this value belongs to the exact-byte domain.
    #[must_use]
    pub const fn is_byte(self) -> bool {
        self.0 & UNICODE_SCALAR_TAG == 0
    }

    /// Returns whether this value belongs to the Unicode-scalar domain.
    #[must_use]
    pub const fn is_unicode_scalar(self) -> bool {
        !self.is_byte()
    }

    /// Extracts an exact byte, rejecting Unicode-domain values.
    pub fn to_byte(self) -> Result<u8, CharacterCodeError> {
        if self.is_byte() {
            u8::try_from(self.0).map_err(|_| CharacterCodeError::InvalidEncoding(self.0))
        } else {
            Err(CharacterCodeError::ExpectedByte)
        }
    }

    /// Extracts the Unicode scalar integer, rejecting byte-domain values.
    pub fn to_unicode_scalar(self) -> Result<u32, CharacterCodeError> {
        if self.is_unicode_scalar() {
            let scalar = self.0 & !UNICODE_SCALAR_TAG;
            char::from_u32(scalar)
                .map(|_| scalar)
                .ok_or(CharacterCodeError::InvalidEncoding(self.0))
        } else {
            Err(CharacterCodeError::ExpectedUnicodeScalar)
        }
    }

    /// Extracts a Rust scalar value, rejecting byte-domain values.
    pub fn to_char(self) -> Result<char, CharacterCodeError> {
        let scalar = self.to_unicode_scalar()?;
        char::from_u32(scalar).ok_or(CharacterCodeError::InvalidEncoding(self.0))
    }

    /// Encodes the tagged semantic value deterministically.
    #[must_use]
    pub const fn to_stable_bytes(self) -> [u8; Self::STABLE_ENCODED_LEN] {
        self.0.to_le_bytes()
    }

    /// Decodes and validates a deterministic tagged semantic value.
    pub fn from_stable_bytes(
        bytes: [u8; Self::STABLE_ENCODED_LEN],
    ) -> Result<Self, CharacterCodeError> {
        let encoded = u32::from_le_bytes(bytes);
        if encoded & UNICODE_SCALAR_TAG == 0 {
            return u8::try_from(encoded)
                .map(Self::from_byte)
                .map_err(|_| CharacterCodeError::InvalidEncoding(encoded));
        }
        let scalar = encoded & !UNICODE_SCALAR_TAG;
        Self::from_unicode_scalar(scalar).map_err(|_| CharacterCodeError::InvalidEncoding(encoded))
    }
}

impl From<u8> for CharacterCode {
    fn from(value: u8) -> Self {
        Self::from_byte(value)
    }
}

impl From<char> for CharacterCode {
    fn from(value: char) -> Self {
        Self(UNICODE_SCALAR_TAG | u32::from(value))
    }
}

impl fmt::Debug for CharacterCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(byte) = self.to_byte() {
            write!(formatter, "CharacterCode::Byte({byte:#04x})")
        } else if let Ok(scalar) = self.to_unicode_scalar() {
            write!(formatter, "CharacterCode::Unicode(U+{scalar:04X})")
        } else {
            write!(formatter, "CharacterCode::Invalid({:#010x})", self.0)
        }
    }
}

impl fmt::Display for CharacterCodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUnicodeScalar(scalar) => {
                write!(formatter, "{scalar:#x} is not a Unicode scalar value")
            }
            Self::ExpectedUnicodeScalar => formatter.write_str("character code is an exact byte"),
            Self::ExpectedByte => formatter.write_str("character code is a Unicode scalar"),
            Self::InvalidEncoding(encoded) => {
                write!(formatter, "invalid character-code encoding {encoded:#010x}")
            }
        }
    }
}

impl std::error::Error for CharacterCodeError {}

/// Projects either canonical input-character domain into the scalar spelling
/// used by [`tex_state::token::Token`].
///
/// TeX82's byte characters occupy the corresponding U+0000..U+00FF scalar
/// values; Unicode profiles retain their scalar unchanged.
pub(crate) fn token_character(code: CharacterCode) -> char {
    match code.to_byte() {
        Ok(byte) => char::from(byte),
        Err(_) => code
            .to_char()
            .expect("registered Unicode source supplies valid scalars"),
    }
}

/// Canonical command family installed before a job begins.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum CommandDialect {
    /// Knuth TeX82.
    #[default]
    Tex82 = 0,
    /// e-TeX 2.6.
    Etex26 = 1,
    /// pdfTeX 1.40.29.
    Pdftex14029 = 2,
}

/// Character domain fixed for the complete lifetime of a job.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum CharacterMode {
    /// Canonical byte-oriented input with codes `0..=255`.
    #[default]
    EightBitExact = 0,
    /// Umber's separately identified Unicode-scalar extension.
    UnicodeExtended = 1,
}

/// Semantic facilities selected by a command dialect and character mode.
///
/// These capabilities are derived entirely from immutable profile data. Host
/// callbacks and host policy are intentionally represented elsewhere.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CommandCapabilities {
    etex: bool,
    pdftex: bool,
    unicode: bool,
}

/// Canonical engine implementation whose compiled semantics execute a job.
///
/// This is distinct from [`CommandProfile`]: pdfTeX can load a format whose
/// command family and frozen state are TeX82 while retaining pdfTeX's changes
/// to shared compiled routines such as pdftex.web §459's dimension scanner.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum CommandEngineSemantics {
    #[default]
    Tex82,
    Etex26,
    Pdftex14029,
}

impl CommandEngineSemantics {
    /// Selects the native canonical implementation for a command profile.
    #[must_use]
    pub const fn for_profile(profile: CommandProfile) -> Self {
        match profile.dialect() {
            CommandDialect::Tex82 => Self::Tex82,
            CommandDialect::Etex26 => Self::Etex26,
            CommandDialect::Pdftex14029 => Self::Pdftex14029,
        }
    }

    /// Whether this implementation contains the requested command family.
    #[must_use]
    pub const fn supports(self, profile: CommandProfile) -> bool {
        matches!(
            (self, profile.dialect()),
            (Self::Tex82, CommandDialect::Tex82)
                | (Self::Etex26, CommandDialect::Tex82 | CommandDialect::Etex26)
                | (Self::Pdftex14029, _)
        )
    }

    /// Whether shared compiled routines use pdfTeX 1.40.29 semantics.
    #[must_use]
    pub const fn supports_pdftex(self) -> bool {
        matches!(self, Self::Pdftex14029)
    }
}

impl CommandCapabilities {
    /// Whether the e-TeX command family is installed.
    #[must_use]
    pub const fn supports_etex(self) -> bool {
        self.etex
    }

    /// Whether the pdfTeX command family is installed.
    #[must_use]
    pub const fn supports_pdftex(self) -> bool {
        self.pdftex
    }

    /// Whether Unicode-scalar character semantics are installed.
    #[must_use]
    pub const fn supports_unicode(self) -> bool {
        self.unicode
    }

    const fn stable_bits(self) -> u8 {
        (self.etex as u8) | ((self.pdftex as u8) << 1) | ((self.unicode as u8) << 2)
    }
}

/// Immutable semantic profile retained for a complete command job.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandProfile {
    dialect: CommandDialect,
    characters: CharacterMode,
}

/// A stable, versioned identity for command-profile-dependent data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandProfileFingerprint(u64);

/// Persistent boundary at which profile compatibility is validated.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandProfileBoundary {
    /// Executor-step rollback snapshot.
    Snapshot,
    /// Durable quiescent command summary.
    Summary,
    /// Portable format image.
    Format,
    /// Aggregate incremental checkpoint.
    Checkpoint,
}

/// A profile-dependent value was presented to a different job profile.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CommandProfileMismatch {
    boundary: CommandProfileBoundary,
    expected: CommandProfileFingerprint,
    found: CommandProfileFingerprint,
}

/// A rejected stable command-profile encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandProfileEncodingError {
    /// The profile encoding version is not supported.
    UnsupportedVersion(u8),
    /// The dialect tag is not recognized.
    UnknownDialect(u8),
    /// The character-mode tag is not recognized.
    UnknownCharacterMode(u8),
    /// Serialized derived capabilities do not match the profile.
    CapabilityMismatch { expected: u8, found: u8 },
}

impl CommandProfile {
    /// Stable encoded width used by [`Self::to_stable_bytes`].
    pub const STABLE_ENCODED_LEN: usize = 4;

    /// Exact Knuth TeX82 compatibility profile.
    pub const TEX82: Self = Self::exact(CommandDialect::Tex82);
    /// Exact e-TeX 2.6 compatibility profile.
    pub const ETEX26: Self = Self::exact(CommandDialect::Etex26);
    /// Exact pdfTeX 1.40.29 compatibility profile.
    pub const PDFTEX14029: Self = Self::exact(CommandDialect::Pdftex14029);

    /// Constructs an exact eight-bit profile for a canonical dialect.
    #[must_use]
    pub const fn exact(dialect: CommandDialect) -> Self {
        Self {
            dialect,
            characters: CharacterMode::EightBitExact,
        }
    }

    /// Constructs the separately identified Unicode extension for a dialect.
    #[must_use]
    pub const fn unicode_extended(dialect: CommandDialect) -> Self {
        Self {
            dialect,
            characters: CharacterMode::UnicodeExtended,
        }
    }

    /// Returns the canonical command family.
    #[must_use]
    pub const fn dialect(self) -> CommandDialect {
        self.dialect
    }

    /// Returns the fixed job character domain.
    #[must_use]
    pub const fn character_mode(self) -> CharacterMode {
        self.characters
    }

    /// Encodes one semantic character for externally committed text output.
    ///
    /// Exact profiles preserve TeX82's byte domain one-for-one. Unicode
    /// profiles use UTF-8 and reject byte-domain values, keeping the two
    /// policies explicit rather than silently sharing a scalar projection.
    pub fn encode_output_character(
        self,
        code: CharacterCode,
    ) -> Result<Vec<u8>, CharacterCodeError> {
        match self.characters {
            CharacterMode::EightBitExact => Ok(vec![code.to_byte()?]),
            CharacterMode::UnicodeExtended => {
                let character = code.to_char()?;
                let mut encoded = [0; 4];
                Ok(character.encode_utf8(&mut encoded).as_bytes().to_vec())
            }
        }
    }

    /// Encodes token-display text for an externally committed TeX output.
    ///
    /// Token display uses Rust scalars as its compact spelling representation
    /// in both character modes. Exact-byte jobs map
    /// `U+0000..=U+00FF` back to their one-byte TeX character codes here;
    /// Unicode jobs encode their scalar spellings as UTF-8. Keeping this at
    /// the immutable profile boundary prevents an exact byte such as `0xC3`
    /// from being encoded a second time merely because token display used a
    /// Rust [`String`].
    pub fn encode_output_text(self, text: &str) -> Result<Vec<u8>, CharacterCodeError> {
        let mut output = Vec::with_capacity(text.len());
        match self.characters {
            CharacterMode::EightBitExact => {
                for character in text.chars() {
                    output.push(
                        u8::try_from(u32::from(character))
                            .map_err(|_| CharacterCodeError::ExpectedByte)?,
                    );
                }
            }
            CharacterMode::UnicodeExtended => output.extend_from_slice(text.as_bytes()),
        }
        Ok(output)
    }

    /// Returns semantic capabilities derived from this immutable profile.
    #[must_use]
    pub const fn capabilities(self) -> CommandCapabilities {
        CommandCapabilities {
            etex: matches!(
                self.dialect,
                CommandDialect::Etex26 | CommandDialect::Pdftex14029
            ),
            pdftex: matches!(self.dialect, CommandDialect::Pdftex14029),
            unicode: matches!(self.characters, CharacterMode::UnicodeExtended),
        }
    }

    /// Encodes all immutable profile data in a deterministic stable form.
    #[must_use]
    pub const fn to_stable_bytes(self) -> [u8; Self::STABLE_ENCODED_LEN] {
        [
            PROFILE_ENCODING_VERSION,
            self.dialect as u8,
            self.characters as u8,
            self.capabilities().stable_bits(),
        ]
    }

    /// Decodes stable profile data and validates its derived capabilities.
    pub fn from_stable_bytes(
        bytes: [u8; Self::STABLE_ENCODED_LEN],
    ) -> Result<Self, CommandProfileEncodingError> {
        if bytes[0] != PROFILE_ENCODING_VERSION {
            return Err(CommandProfileEncodingError::UnsupportedVersion(bytes[0]));
        }
        let dialect = match bytes[1] {
            0 => CommandDialect::Tex82,
            1 => CommandDialect::Etex26,
            2 => CommandDialect::Pdftex14029,
            tag => return Err(CommandProfileEncodingError::UnknownDialect(tag)),
        };
        let characters = match bytes[2] {
            0 => CharacterMode::EightBitExact,
            1 => CharacterMode::UnicodeExtended,
            tag => return Err(CommandProfileEncodingError::UnknownCharacterMode(tag)),
        };
        let profile = Self {
            dialect,
            characters,
        };
        let expected = profile.capabilities().stable_bits();
        if bytes[3] != expected {
            return Err(CommandProfileEncodingError::CapabilityMismatch {
                expected,
                found: bytes[3],
            });
        }
        Ok(profile)
    }

    /// Computes the deterministic, domain-separated profile fingerprint.
    #[must_use]
    pub fn fingerprint(self) -> CommandProfileFingerprint {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in PROFILE_FINGERPRINT_DOMAIN
            .iter()
            .copied()
            .chain(self.to_stable_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        CommandProfileFingerprint(hash)
    }

    pub(crate) fn validate_fingerprint(
        self,
        boundary: CommandProfileBoundary,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        let expected = self.fingerprint();
        if expected == found {
            Ok(())
        } else {
            Err(CommandProfileMismatch {
                boundary,
                expected,
                found,
            })
        }
    }
}

impl Default for CommandProfileFingerprint {
    fn default() -> Self {
        CommandProfile::default().fingerprint()
    }
}

impl CommandProfileFingerprint {
    /// Returns the stable integer representation used in format/checkpoint IDs.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Reconstructs a fingerprint read from a persistent identity.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

impl CommandProfileMismatch {
    /// Returns the boundary that rejected the identity.
    #[must_use]
    pub const fn boundary(self) -> CommandProfileBoundary {
        self.boundary
    }

    /// Returns the active job's required identity.
    #[must_use]
    pub const fn expected(self) -> CommandProfileFingerprint {
        self.expected
    }

    /// Returns the incompatible presented identity.
    #[must_use]
    pub const fn found(self) -> CommandProfileFingerprint {
        self.found
    }
}

impl fmt::Display for CommandProfileMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} command-profile mismatch: expected {:#018x}, found {:#018x}",
            self.boundary,
            self.expected.get(),
            self.found.get()
        )
    }
}

impl std::error::Error for CommandProfileMismatch {}

impl fmt::Display for CommandProfileEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported command-profile version {version}")
            }
            Self::UnknownDialect(tag) => write!(formatter, "unknown command dialect tag {tag}"),
            Self::UnknownCharacterMode(tag) => {
                write!(formatter, "unknown command character-mode tag {tag}")
            }
            Self::CapabilityMismatch { expected, found } => write!(
                formatter,
                "command-profile capability mismatch: expected {expected:#04x}, found {found:#04x}"
            ),
        }
    }
}

impl std::error::Error for CommandProfileEncodingError {}

#[cfg(test)]
mod tests;
