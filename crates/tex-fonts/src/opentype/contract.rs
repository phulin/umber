use std::fmt;

use sha2::{Digest, Sha256};

/// Version of the canonical decoded-table identity policy.
pub const FONT_PROGRAM_IDENTITY_VERSION: u8 = 1;
/// Version of the font-instance identity policy.
pub const FONT_INSTANCE_IDENTITY_VERSION: u8 = 1;

/// A four-byte OpenType table, feature, script, language, or variation tag.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpenTypeTag([u8; 4]);

impl OpenTypeTag {
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for OpenTypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{}", char::from(byte))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum FontContainer {
    OpenType = 1,
    TrueType = 2,
    Collection = 3,
    Woff2 = 4,
}

impl FontContainer {
    #[must_use]
    pub const fn mask(self) -> u8 {
        1 << ((self as u8) - 1)
    }
}

/// Accepted transport containers for one execution boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AcceptedFontContainers(u8);

impl AcceptedFontContainers {
    pub const NATIVE: Self = Self(FontContainer::OpenType.mask() | FontContainer::TrueType.mask());
    pub const NATIVE_WITH_COLLECTIONS: Self = Self(
        FontContainer::OpenType.mask()
            | FontContainer::TrueType.mask()
            | FontContainer::Collection.mask(),
    );
    pub const WASM: Self = Self(FontContainer::Woff2.mask());

    #[must_use]
    pub const fn contains(self, container: FontContainer) -> bool {
        self.0 & container.mask() != 0
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    fn from_bits(bits: u8) -> Result<Self, FontWireError> {
        if bits == 0 || bits & !0x0f != 0 {
            Err(FontWireError::InvalidContainerMask(bits))
        } else {
            Ok(Self(bits))
        }
    }
}

/// Reasons the engine currently needs a selected font.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontPurposes(u8);

impl FontPurposes {
    pub const LAYOUT: Self = Self(1);
    pub const HTML: Self = Self(2);
    pub const LAYOUT_AND_HTML: Self = Self(3);

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    fn from_bits(bits: u8) -> Result<Self, FontWireError> {
        if bits == 0 || bits & !3 != 0 {
            Err(FontWireError::InvalidPurposeMask(bits))
        } else {
            Ok(Self(bits))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VariationCoordinate {
    pub tag: OpenTypeTag,
    /// Selected coordinate in signed 16.16 fixed-point font-axis units.
    pub value: i32,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VariationSelection(Vec<VariationCoordinate>);

impl VariationSelection {
    pub fn new(mut coordinates: Vec<VariationCoordinate>) -> Result<Self, FontSelectionError> {
        coordinates.sort_unstable_by_key(|coordinate| coordinate.tag);
        if coordinates
            .windows(2)
            .any(|pair| pair[0].tag == pair[1].tag)
        {
            return Err(FontSelectionError::DuplicateVariationAxis);
        }
        if coordinates.len() > FontLimits::HARD_MAX.max_variation_axes {
            return Err(FontSelectionError::TooManyVariationAxes(coordinates.len()));
        }
        Ok(Self(coordinates))
    }

    #[must_use]
    pub fn coordinates(&self) -> &[VariationCoordinate] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FeatureSetting {
    pub tag: OpenTypeTag,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontFeaturePolicy(Vec<FeatureSetting>);

impl FontFeaturePolicy {
    pub fn new(mut settings: Vec<FeatureSetting>) -> Result<Self, FontSelectionError> {
        settings.sort_unstable_by_key(|setting| setting.tag);
        if settings.windows(2).any(|pair| pair[0].tag == pair[1].tag) {
            return Err(FontSelectionError::DuplicateFeature);
        }
        if settings.len() > FontLimits::HARD_MAX.max_features {
            return Err(FontSelectionError::TooManyFeatures(settings.len()));
        }
        Ok(Self(settings))
    }

    #[must_use]
    pub fn settings(&self) -> &[FeatureSetting] {
        &self.0
    }
}

impl Default for FontFeaturePolicy {
    fn default() -> Self {
        Self(vec![
            FeatureSetting {
                tag: OpenTypeTag::new(*b"kern"),
                enabled: true,
            },
            FeatureSetting {
                tag: OpenTypeTag::new(*b"liga"),
                enabled: true,
            },
        ])
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum WritingDirection {
    LeftToRight = 1,
    RightToLeft = 2,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontRequestKey {
    logical_name: String,
    pub face_index: u32,
    pub variation: VariationSelection,
    pub feature_policy: FontFeaturePolicy,
}

impl FontRequestKey {
    pub fn new(
        logical_name: impl Into<String>,
        face_index: u32,
        variation: VariationSelection,
        feature_policy: FontFeaturePolicy,
    ) -> Result<Self, FontSelectionError> {
        let logical_name = logical_name.into();
        if logical_name.is_empty() {
            return Err(FontSelectionError::EmptyLogicalName);
        }
        if logical_name.len() > FontLimits::HARD_MAX.max_logical_name_bytes {
            return Err(FontSelectionError::LogicalNameTooLong(logical_name.len()));
        }
        if logical_name.chars().any(char::is_control) {
            return Err(FontSelectionError::ControlInLogicalName);
        }
        if face_index >= FontLimits::HARD_MAX.max_faces as u32 {
            return Err(FontSelectionError::FaceIndexTooLarge(face_index));
        }
        Ok(Self {
            logical_name,
            face_index,
            variation,
            feature_policy,
        })
    }

    #[must_use]
    pub fn logical_name(&self) -> &str {
        &self.logical_name
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontRequest {
    pub key: FontRequestKey,
    pub accepted_containers: AcceptedFontContainers,
    pub purposes: FontPurposes,
}

impl FontRequest {
    /// Canonical versioned request encoding shared by native and WASM fixtures.
    #[must_use]
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = b"UFRQ\x01".to_vec();
        encode_key(&self.key, &mut out);
        out.push(self.accepted_containers.bits());
        out.push(self.purposes.bits());
        out
    }

    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, FontWireError> {
        let mut input = WireReader::new(bytes, b"UFRQ\x01")?;
        let key = decode_key(&mut input)?;
        let accepted_containers = AcceptedFontContainers::from_bits(input.byte()?)?;
        let purposes = FontPurposes::from_bits(input.byte()?)?;
        input.finish()?;
        Ok(Self {
            key,
            accepted_containers,
            purposes,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontObjectIdentity([u8; 32]);

impl FontObjectIdentity {
    #[must_use]
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontProgramIdentity([u8; 32]);

impl FontProgramIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontInstanceIdentity([u8; 32]);

impl FontInstanceIdentity {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn new(
        program: FontProgramIdentity,
        face_index: u32,
        size_sp: i32,
        variation: &VariationSelection,
        features: &FontFeaturePolicy,
        direction: WritingDirection,
    ) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"umber.font-instance");
        hash.update([FONT_INSTANCE_IDENTITY_VERSION]);
        hash.update(program.bytes());
        hash.update(face_index.to_be_bytes());
        hash.update(size_sp.to_be_bytes());
        hash.update([direction as u8, 0]); // synthetic styles are always prohibited
        hash.update((variation.coordinates().len() as u32).to_be_bytes());
        for coordinate in variation.coordinates() {
            hash.update(coordinate.tag.bytes());
            hash.update(coordinate.value.to_be_bytes());
        }
        hash.update((features.settings().len() as u32).to_be_bytes());
        for feature in features.settings() {
            hash.update(feature.tag.bytes());
            hash.update([u8::from(feature.enabled)]);
        }
        Self(hash.finalize().into())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFont {
    pub request: FontRequestKey,
    pub container: FontContainer,
    pub bytes: Vec<u8>,
    pub declared_object_sha256: Option<FontObjectIdentity>,
    pub declared_program_identity: Option<FontProgramIdentity>,
    pub provenance: Option<String>,
}

impl ResolvedFont {
    /// Canonical versioned response encoding. The resource bytes remain binary.
    #[must_use]
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = b"UFRS\x01".to_vec();
        encode_key(&self.request, &mut out);
        out.push(self.container as u8);
        encode_optional_identity(
            self.declared_object_sha256.map(FontObjectIdentity::bytes),
            &mut out,
        );
        encode_optional_identity(
            self.declared_program_identity
                .map(FontProgramIdentity::bytes),
            &mut out,
        );
        encode_optional_string(self.provenance.as_deref(), &mut out);
        encode_bytes(&self.bytes, &mut out);
        out
    }

    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, FontWireError> {
        let mut input = WireReader::new(bytes, b"UFRS\x01")?;
        let request = decode_key(&mut input)?;
        let container = match input.byte()? {
            1 => FontContainer::OpenType,
            2 => FontContainer::TrueType,
            3 => FontContainer::Collection,
            4 => FontContainer::Woff2,
            value => return Err(FontWireError::InvalidContainer(value)),
        };
        let declared_object_sha256 =
            decode_optional_identity(&mut input)?.map(FontObjectIdentity::from_bytes);
        let declared_program_identity =
            decode_optional_identity(&mut input)?.map(FontProgramIdentity::from_bytes);
        let provenance = decode_optional_string(&mut input)?;
        let bytes = input.bytes()?.to_vec();
        input.finish()?;
        Ok(Self {
            request,
            container,
            bytes,
            declared_object_sha256,
            declared_program_identity,
            provenance,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontLimits {
    pub max_object_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_tables: usize,
    pub max_faces: usize,
    pub max_glyphs: usize,
    pub max_mappings: usize,
    pub max_variation_axes: usize,
    pub max_features: usize,
    pub max_logical_name_bytes: usize,
    pub max_provenance_bytes: usize,
}

impl FontLimits {
    pub const HARD_MAX: Self = Self {
        max_object_bytes: 64 * 1024 * 1024,
        max_decoded_bytes: 128 * 1024 * 1024,
        max_tables: 4096,
        max_faces: 1024,
        max_glyphs: 1_000_000,
        max_mappings: 1_200_000,
        max_variation_axes: 64,
        max_features: 256,
        max_logical_name_bytes: 1024,
        max_provenance_bytes: 4096,
    };
}

impl Default for FontLimits {
    fn default() -> Self {
        Self {
            max_object_bytes: 16 * 1024 * 1024,
            max_decoded_bytes: 32 * 1024 * 1024,
            max_tables: 512,
            max_faces: 64,
            max_glyphs: 131_072,
            max_mappings: 262_144,
            max_variation_axes: 32,
            max_features: 128,
            max_logical_name_bytes: 255,
            max_provenance_bytes: 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontSelectionError {
    EmptyLogicalName,
    LogicalNameTooLong(usize),
    ControlInLogicalName,
    FaceIndexTooLarge(u32),
    DuplicateVariationAxis,
    TooManyVariationAxes(usize),
    DuplicateFeature,
    TooManyFeatures(usize),
}

impl fmt::Display for FontSelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid font selection: {self:?}")
    }
}

impl std::error::Error for FontSelectionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontWireError {
    UnsupportedVersion,
    Truncated,
    TrailingBytes,
    InvalidUtf8,
    InvalidBoolean(u8),
    InvalidContainer(u8),
    InvalidContainerMask(u8),
    InvalidPurposeMask(u8),
    InvalidSelection(FontSelectionError),
    LengthOverflow,
}

impl fmt::Display for FontWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid font resource encoding: {self:?}")
    }
}

impl std::error::Error for FontWireError {}

fn encode_key(key: &FontRequestKey, out: &mut Vec<u8>) {
    encode_bytes(key.logical_name.as_bytes(), out);
    out.extend_from_slice(&key.face_index.to_be_bytes());
    out.extend_from_slice(&(key.variation.coordinates().len() as u16).to_be_bytes());
    for coordinate in key.variation.coordinates() {
        out.extend_from_slice(&coordinate.tag.bytes());
        out.extend_from_slice(&coordinate.value.to_be_bytes());
    }
    out.extend_from_slice(&(key.feature_policy.settings().len() as u16).to_be_bytes());
    for feature in key.feature_policy.settings() {
        out.extend_from_slice(&feature.tag.bytes());
        out.push(u8::from(feature.enabled));
    }
}

fn decode_key(input: &mut WireReader<'_>) -> Result<FontRequestKey, FontWireError> {
    let logical_name = std::str::from_utf8(input.bytes()?)
        .map_err(|_| FontWireError::InvalidUtf8)?
        .to_owned();
    let face_index = input.u32()?;
    let variation_count = usize::from(input.u16()?);
    let mut variation =
        Vec::with_capacity(variation_count.min(FontLimits::HARD_MAX.max_variation_axes));
    for _ in 0..variation_count {
        variation.push(VariationCoordinate {
            tag: OpenTypeTag::new(input.array()?),
            value: input.i32()?,
        });
    }
    let feature_count = usize::from(input.u16()?);
    let mut features = Vec::with_capacity(feature_count.min(FontLimits::HARD_MAX.max_features));
    for _ in 0..feature_count {
        let tag = OpenTypeTag::new(input.array()?);
        let enabled = match input.byte()? {
            0 => false,
            1 => true,
            value => return Err(FontWireError::InvalidBoolean(value)),
        };
        features.push(FeatureSetting { tag, enabled });
    }
    let variation = VariationSelection::new(variation).map_err(FontWireError::InvalidSelection)?;
    let feature_policy =
        FontFeaturePolicy::new(features).map_err(FontWireError::InvalidSelection)?;
    FontRequestKey::new(logical_name, face_index, variation, feature_policy)
        .map_err(FontWireError::InvalidSelection)
}

fn encode_optional_identity(identity: Option<[u8; 32]>, out: &mut Vec<u8>) {
    out.push(u8::from(identity.is_some()));
    if let Some(identity) = identity {
        out.extend_from_slice(&identity);
    }
}

fn decode_optional_identity(input: &mut WireReader<'_>) -> Result<Option<[u8; 32]>, FontWireError> {
    match input.byte()? {
        0 => Ok(None),
        1 => Ok(Some(input.array()?)),
        value => Err(FontWireError::InvalidBoolean(value)),
    }
}

fn encode_optional_string(value: Option<&str>, out: &mut Vec<u8>) {
    out.push(u8::from(value.is_some()));
    if let Some(value) = value {
        encode_bytes(value.as_bytes(), out);
    }
}

fn decode_optional_string(input: &mut WireReader<'_>) -> Result<Option<String>, FontWireError> {
    match input.byte()? {
        0 => Ok(None),
        1 => Ok(Some(
            std::str::from_utf8(input.bytes()?)
                .map_err(|_| FontWireError::InvalidUtf8)?
                .to_owned(),
        )),
        value => Err(FontWireError::InvalidBoolean(value)),
    }
}

fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

struct WireReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8], magic: &[u8]) -> Result<Self, FontWireError> {
        if bytes.get(..magic.len()) != Some(magic) {
            return Err(FontWireError::UnsupportedVersion);
        }
        Ok(Self {
            bytes,
            offset: magic.len(),
        })
    }
    fn byte(&mut self) -> Result<u8, FontWireError> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or(FontWireError::Truncated)?;
        self.offset += 1;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], FontWireError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(FontWireError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(FontWireError::Truncated)?
            .try_into()
            .map_err(|_| FontWireError::Truncated)?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, FontWireError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, FontWireError> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32, FontWireError> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], FontWireError> {
        let len = usize::try_from(self.u32()?).map_err(|_| FontWireError::LengthOverflow)?;
        let end = self
            .offset
            .checked_add(len)
            .ok_or(FontWireError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(FontWireError::Truncated)?;
        self.offset = end;
        Ok(value)
    }
    fn finish(self) -> Result<(), FontWireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(FontWireError::TrailingBytes)
        }
    }
}
