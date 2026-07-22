use std::fmt;

use sha2::{Digest, Sha256};

/// Version of the canonical decoded-table identity policy.
pub const FONT_PROGRAM_IDENTITY_VERSION: u8 = 1;
/// Version of the font-instance identity policy.
pub const FONT_INSTANCE_IDENTITY_VERSION: u8 = 2;
/// Versioned semantics for OpenType feature overrides.
pub const FONT_FEATURE_POLICY_VERSION: u8 = 1;

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

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0[0] >= 0x20
            && self.0[0] <= 0x7e
            && self.0[1] >= 0x20
            && self.0[1] <= 0x7e
            && self.0[2] >= 0x20
            && self.0[2] <= 0x7e
            && self.0[3] >= 0x20
            && self.0[3] <= 0x7e
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VariationInstance {
    #[default]
    Default,
    /// Selects an `fvar` instance by its subfamily name identifier.
    Named(u16),
    Coordinates,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VariationSelection {
    instance: VariationInstance,
    coordinates: Vec<VariationCoordinate>,
}

impl VariationSelection {
    pub fn new(mut coordinates: Vec<VariationCoordinate>) -> Result<Self, FontSelectionError> {
        if coordinates
            .iter()
            .any(|coordinate| !coordinate.tag.is_valid())
        {
            return Err(FontSelectionError::InvalidTag);
        }
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
        Ok(Self {
            instance: if coordinates.is_empty() {
                VariationInstance::Default
            } else {
                VariationInstance::Coordinates
            },
            coordinates,
        })
    }

    #[must_use]
    pub const fn named(subfamily_name_id: u16) -> Self {
        Self {
            instance: VariationInstance::Named(subfamily_name_id),
            coordinates: Vec::new(),
        }
    }

    pub fn resolved_named(
        subfamily_name_id: u16,
        coordinates: Vec<VariationCoordinate>,
    ) -> Result<Self, FontSelectionError> {
        let mut selection = Self::new(coordinates)?;
        selection.instance = VariationInstance::Named(subfamily_name_id);
        Ok(selection)
    }

    #[must_use]
    pub const fn instance(&self) -> VariationInstance {
        self.instance
    }

    #[must_use]
    pub fn coordinates(&self) -> &[VariationCoordinate] {
        &self.coordinates
    }

    pub(crate) fn with_resolved_coordinates(
        mut self,
        coordinates: Vec<VariationCoordinate>,
    ) -> Self {
        self.coordinates = coordinates;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FeatureSetting {
    pub tag: OpenTypeTag,
    /// OpenType feature value. Zero disables the feature; non-zero values may
    /// select alternates for features such as `salt`, `ss01`, and `cv01`.
    pub value: u32,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontFeaturePolicy(Vec<FeatureSetting>);

impl FontFeaturePolicy {
    pub fn new(mut settings: Vec<FeatureSetting>) -> Result<Self, FontSelectionError> {
        if settings.iter().any(|setting| !setting.tag.is_valid()) {
            return Err(FontSelectionError::InvalidTag);
        }
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
                value: 1,
            },
            FeatureSetting {
                tag: OpenTypeTag::new(*b"liga"),
                value: 1,
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

/// Canonical BCP-47 language input for OpenType language-system selection.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontLanguage(String);

impl FontLanguage {
    pub fn new(value: impl Into<String>) -> Result<Self, FontSelectionError> {
        let value = value.into().to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 63
            || value.starts_with('-')
            || value.ends_with('-')
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
        {
            return Err(FontSelectionError::InvalidLanguage(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontRequestKey {
    logical_name: String,
    pub face_index: u32,
    pub variation: VariationSelection,
    pub feature_policy: FontFeaturePolicy,
    pub direction: WritingDirection,
    pub script: Option<OpenTypeTag>,
    pub language: Option<FontLanguage>,
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
            direction: WritingDirection::LeftToRight,
            script: None,
            language: None,
        })
    }

    /// Adds the shaping context that participates in request and instance
    /// identity. `None` script keeps deterministic Unicode-script inference;
    /// `None` language selects HarfBuzz's language-neutral behavior.
    pub fn with_shaping_context(
        mut self,
        direction: WritingDirection,
        script: Option<OpenTypeTag>,
        language: Option<FontLanguage>,
    ) -> Result<Self, FontSelectionError> {
        if script.is_some_and(|tag| !tag.is_valid()) {
            return Err(FontSelectionError::InvalidTag);
        }
        self.direction = direction;
        self.script = script;
        self.language = language;
        Ok(self)
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
        let mut out = b"UFRQ\x02".to_vec();
        encode_key(&self.key, &mut out);
        out.push(self.accepted_containers.bits());
        out.push(self.purposes.bits());
        out
    }

    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, FontWireError> {
        let mut input = WireReader::new(bytes, b"UFRQ\x02")?;
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

#[derive(Clone, Copy, Debug)]
pub struct FontInstanceContext<'a> {
    pub variation: &'a VariationSelection,
    pub features: &'a FontFeaturePolicy,
    pub direction: WritingDirection,
    pub script: Option<OpenTypeTag>,
    pub language: Option<&'a FontLanguage>,
}

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
        Self::new_with_context(
            program,
            face_index,
            size_sp,
            FontInstanceContext {
                variation,
                features,
                direction,
                script: None,
                language: None,
            },
        )
    }

    #[must_use]
    pub fn new_with_context(
        program: FontProgramIdentity,
        face_index: u32,
        size_sp: i32,
        context: FontInstanceContext<'_>,
    ) -> Self {
        let FontInstanceContext {
            variation,
            features,
            direction,
            script,
            language,
        } = context;
        let mut hash = Sha256::new();
        hash.update(b"umber.font-instance");
        hash.update([FONT_INSTANCE_IDENTITY_VERSION]);
        hash.update(program.bytes());
        hash.update(face_index.to_be_bytes());
        hash.update(size_sp.to_be_bytes());
        hash.update([direction as u8, 0, 0]); // synthetic styles and optical sizing are prohibited
        hash.update([match variation.instance() {
            VariationInstance::Default => 0,
            VariationInstance::Named(_) => 1,
            VariationInstance::Coordinates => 2,
        }]);
        if let VariationInstance::Named(name_id) = variation.instance() {
            hash.update(name_id.to_be_bytes());
        }
        hash.update((variation.coordinates().len() as u32).to_be_bytes());
        for coordinate in variation.coordinates() {
            hash.update(coordinate.tag.bytes());
            hash.update(coordinate.value.to_be_bytes());
        }
        hash.update([FONT_FEATURE_POLICY_VERSION]);
        hash.update((features.settings().len() as u32).to_be_bytes());
        for feature in features.settings() {
            hash.update(feature.tag.bytes());
            hash.update(feature.value.to_be_bytes());
        }
        hash.update(script.map_or([0; 4], OpenTypeTag::bytes));
        let language = language.map_or("", FontLanguage::as_str).as_bytes();
        hash.update((language.len() as u32).to_be_bytes());
        hash.update(language);
        Self(hash.finalize().into())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyFontMapping {
    /// SHA-256 identity of the exact TFM object whose byte codes are mapped.
    pub tfm_sha256: [u8; 32],
    /// Exactly 256 entries; absent entries are not renderable through this mapping.
    pub encoding: Vec<Option<String>>,
    /// The client has affirmatively authorized embedding the supplied font object.
    pub embeddable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFont {
    pub request: FontRequestKey,
    pub container: FontContainer,
    pub bytes: Vec<u8>,
    pub declared_object_sha256: Option<FontObjectIdentity>,
    pub declared_program_identity: Option<FontProgramIdentity>,
    pub provenance: Option<String>,
    /// Optional exact legacy-code mapping carried by the same typed response.
    pub legacy_mapping: Option<LegacyFontMapping>,
}

impl ResolvedFont {
    /// Canonical versioned response encoding. The resource bytes remain binary.
    #[must_use]
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = b"UFRS\x03".to_vec();
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
        match &self.legacy_mapping {
            None => out.push(0),
            Some(mapping) => {
                out.push(1);
                out.extend_from_slice(&mapping.tfm_sha256);
                out.push(u8::from(mapping.embeddable));
                out.extend_from_slice(&(mapping.encoding.len() as u32).to_be_bytes());
                for entry in &mapping.encoding {
                    encode_optional_string(entry.as_deref(), &mut out);
                }
            }
        }
        encode_bytes(&self.bytes, &mut out);
        out
    }

    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, FontWireError> {
        let mut input = WireReader::new(bytes, b"UFRS\x03")?;
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
        let legacy_mapping = match input.byte()? {
            0 => None,
            1 => {
                let tfm_sha256 = input.array()?;
                let embeddable = match input.byte()? {
                    0 => false,
                    1 => true,
                    value => return Err(FontWireError::InvalidBoolean(value)),
                };
                let count = input.u32()? as usize;
                if count != 256 {
                    return Err(FontWireError::InvalidLegacyMappingCount(count));
                }
                let mut encoding = Vec::with_capacity(count);
                for _ in 0..count {
                    encoding.push(decode_optional_string(&mut input)?);
                }
                Some(LegacyFontMapping {
                    tfm_sha256,
                    encoding,
                    embeddable,
                })
            }
            value => return Err(FontWireError::InvalidBoolean(value)),
        };
        let bytes = input.bytes()?.to_vec();
        input.finish()?;
        Ok(Self {
            request,
            container,
            bytes,
            declared_object_sha256,
            declared_program_identity,
            provenance,
            legacy_mapping,
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
    pub max_math_records: usize,
    pub max_math_assembly_parts: usize,
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
        max_math_records: 4_000_000,
        max_math_assembly_parts: 1_000_000,
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
            max_math_records: 262_144,
            max_math_assembly_parts: 65_536,
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
    InvalidTag,
    InvalidLanguage(String),
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
    InvalidVariationInstance(u8),
    InvalidDirection(u8),
    InvalidSelection(FontSelectionError),
    InvalidLegacyMappingCount(usize),
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
    match key.variation.instance() {
        VariationInstance::Default => out.push(0),
        VariationInstance::Named(name_id) => {
            out.push(1);
            out.extend_from_slice(&name_id.to_be_bytes());
        }
        VariationInstance::Coordinates => out.push(2),
    }
    out.extend_from_slice(&(key.variation.coordinates().len() as u16).to_be_bytes());
    for coordinate in key.variation.coordinates() {
        out.extend_from_slice(&coordinate.tag.bytes());
        out.extend_from_slice(&coordinate.value.to_be_bytes());
    }
    out.extend_from_slice(&(key.feature_policy.settings().len() as u16).to_be_bytes());
    for feature in key.feature_policy.settings() {
        out.extend_from_slice(&feature.tag.bytes());
        out.extend_from_slice(&feature.value.to_be_bytes());
    }
    out.push(key.direction as u8);
    encode_optional_tag(key.script, out);
    encode_optional_string(key.language.as_ref().map(FontLanguage::as_str), out);
}

fn decode_key(input: &mut WireReader<'_>) -> Result<FontRequestKey, FontWireError> {
    let logical_name = std::str::from_utf8(input.bytes()?)
        .map_err(|_| FontWireError::InvalidUtf8)?
        .to_owned();
    let face_index = input.u32()?;
    let instance = match input.byte()? {
        0 => VariationInstance::Default,
        1 => VariationInstance::Named(input.u16()?),
        2 => VariationInstance::Coordinates,
        value => return Err(FontWireError::InvalidVariationInstance(value)),
    };
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
        let value = input.u32()?;
        features.push(FeatureSetting { tag, value });
    }
    let mut variation =
        VariationSelection::new(variation).map_err(FontWireError::InvalidSelection)?;
    variation.instance = instance;
    if !matches!(instance, VariationInstance::Coordinates) && !variation.coordinates.is_empty() {
        return Err(FontWireError::InvalidVariationInstance(255));
    }
    let feature_policy =
        FontFeaturePolicy::new(features).map_err(FontWireError::InvalidSelection)?;
    let direction = match input.byte()? {
        1 => WritingDirection::LeftToRight,
        2 => WritingDirection::RightToLeft,
        value => return Err(FontWireError::InvalidDirection(value)),
    };
    let script = decode_optional_tag(input)?;
    let language = decode_optional_string(input)?
        .map(FontLanguage::new)
        .transpose()
        .map_err(FontWireError::InvalidSelection)?;
    FontRequestKey::new(logical_name, face_index, variation, feature_policy)
        .and_then(|key| key.with_shaping_context(direction, script, language))
        .map_err(FontWireError::InvalidSelection)
}

fn encode_optional_tag(tag: Option<OpenTypeTag>, out: &mut Vec<u8>) {
    out.push(u8::from(tag.is_some()));
    if let Some(tag) = tag {
        out.extend_from_slice(&tag.bytes());
    }
}

fn decode_optional_tag(input: &mut WireReader<'_>) -> Result<Option<OpenTypeTag>, FontWireError> {
    match input.byte()? {
        0 => Ok(None),
        1 => Ok(Some(OpenTypeTag::new(input.array()?))),
        value => Err(FontWireError::InvalidBoolean(value)),
    }
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
