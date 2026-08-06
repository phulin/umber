//! Behavior-free schema for the canonical primitive catalogue.
//!
//! The catalogue describes stable identities and static policy. It deliberately
//! contains no execution callbacks or dispatch enums: handwritten command
//! processing consumes generated views in the neighboring modules.

use std::collections::HashMap;

type WebIdentityKey<'a> = (&'a str, Option<i64>);
type WebIdentityOccurrence<'a> = (PrimitiveProfiles, Option<&'a str>);

/// A stable operand namespace.
///
/// Namespace plus numeric value, rather than a Rust discriminant, is the
/// persistent identity. The exception namespaces cover the primitive meanings
/// that are not represented by the two command enums today.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PrimitiveOperandDomain {
    Expandable,
    Unexpandable,
    InternalInteger,
    IntegerParameter,
    DimensionParameter,
    GlueParameter,
    MathGlueParameter,
    TokenParameter,
    PageDimension,
    PageInteger,
    FontSelector,
    Relax,
    FrozenMacro,
    InaccessibleCommand,
}

/// Catalogue-owned stable identity for one primitive meaning.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PrimitiveOperand {
    pub domain: PrimitiveOperandDomain,
    pub value: u64,
}

impl PrimitiveOperand {
    #[must_use]
    pub const fn new(domain: PrimitiveOperandDomain, value: u64) -> Self {
        Self { domain, value }
    }
}

/// Engine/profile layer in which catalogue metadata applies.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum PrimitiveProfile {
    Tex82 = 1 << 0,
    Etex26 = 1 << 1,
    LatexCompatibility = 1 << 2,
    Pdftex14029 = 1 << 3,
}

/// A compact explicit set of [`PrimitiveProfile`] values.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PrimitiveProfiles(u8);

impl PrimitiveProfiles {
    pub const NONE: Self = Self(0);
    pub const TEX82: Self = Self::only(PrimitiveProfile::Tex82);
    pub const ETEX26: Self = Self::only(PrimitiveProfile::Etex26);
    pub const LATEX_COMPATIBILITY: Self = Self::only(PrimitiveProfile::LatexCompatibility);
    pub const PDFTEX14029: Self = Self::only(PrimitiveProfile::Pdftex14029);
    pub const ALL: Self =
        Self(Self::TEX82.0 | Self::ETEX26.0 | Self::LATEX_COMPATIBILITY.0 | Self::PDFTEX14029.0);

    #[must_use]
    pub const fn only(profile: PrimitiveProfile) -> Self {
        Self(profile as u8)
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn contains(self, profile: PrimitiveProfile) -> bool {
        self.0 & Self::only(profile).0 != 0
    }

    #[must_use]
    pub const fn contains_all(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Whether a spelling is the documented name or an intentional alias.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SpellingKind {
    Canonical,
    Alias,
    Frozen,
}

/// One source spelling and the exact profiles that expose it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PrimitiveSpelling {
    pub name: &'static str,
    pub profiles: PrimitiveProfiles,
    pub kind: SpellingKind,
    pub installation: InstallationPolicy,
}

/// How TeX's mouth classifies the meaning before main control.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExpansionClass {
    Expandable,
    Unexpandable,
    InternalQuantity,
    FrozenMacro,
}

/// A profile-specific `cmd`/`chr` identity from WEB or a change file.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WebIdentity {
    pub profiles: PrimitiveProfiles,
    pub command: &'static str,
    pub operand: Option<i64>,
    /// Explicit equivalence class for distinct catalogue operands that share
    /// one WEB `cmd`/`chr` pair. `None` requires uniqueness.
    pub collision_group: Option<&'static str>,
}

/// Relationship with TeX's prefix loop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PrefixAdmissibility {
    /// The command is not accepted after a prefix.
    Forbidden,
    /// The command is accepted after `\global`, `\long`, or another prefix.
    Admissible,
    /// The primitive is itself a prefix.
    Prefix,
}

/// Fresh-engine and format-restoration installation policy.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InstallationPolicy(u8);

impl InstallationPolicy {
    pub const NONE: Self = Self(0);
    pub const INITEX: Self = Self(1 << 0);
    pub const FORMAT_REGISTRY: Self = Self(1 << 1);
    pub const BOTH: Self = Self(Self::INITEX.0 | Self::FORMAT_REGISTRY.0);

    #[must_use]
    pub const fn contains(self, policy: Self) -> bool {
        self.0 & policy.0 == policy.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Dense state-bank class used by a parameter primitive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParameterBankClass {
    Integer,
    Dimension,
    Glue,
    MathGlue,
    Tokens,
}

/// Stable parameter bank cell named by a primitive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ParameterCell {
    pub class: ParameterBankClass,
    pub index: u16,
}

/// Initial value policy for a parameter cell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParameterDefault {
    Integer(i32),
    Scaled(i32),
    Glue(GlueParameterDefault),
    EmptyTokens,
    /// The host supplies a reproducible job-clock field during initialization.
    JobClock(JobClockField),
}

/// Initial glue value without referring to an allocation identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueParameterDefault {
    pub width: i32,
    pub stretch: i32,
    pub stretch_order: u8,
    pub shrink: i32,
    pub shrink_order: u8,
}

/// Reproducible field selected from the host-provided job clock.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobClockField {
    MinutesSinceMidnight,
    Day,
    Month,
    Year,
}

/// Documentation inventory that owns the primitive row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentationFamily {
    Tex82,
    Etex26,
    LatexCompatibility,
    Pdftex14029,
    FrozenPrivate,
}

/// One declarative primitive record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveDescriptor {
    pub operand: PrimitiveOperand,
    pub spellings: &'static [PrimitiveSpelling],
    pub profiles: PrimitiveProfiles,
    pub expansion: ExpansionClass,
    pub web: &'static [WebIdentity],
    pub prefix: PrefixAdmissibility,
    pub parameter: Option<(ParameterCell, ParameterDefault)>,
    pub documentation: DocumentationFamily,
}

/// Every validation failure found in a catalogue audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogueValidationError {
    EmptyProfiles {
        operand: PrimitiveOperand,
    },
    EmptySpelling {
        operand: PrimitiveOperand,
    },
    SpellingOutsideProfiles {
        operand: PrimitiveOperand,
        name: &'static str,
    },
    WebIdentityOutsideProfiles {
        operand: PrimitiveOperand,
        command: &'static str,
    },
    MissingCanonicalSpelling {
        operand: PrimitiveOperand,
    },
    ParameterDomainMismatch {
        operand: PrimitiveOperand,
        cell: ParameterCell,
    },
    ParameterDefaultMismatch {
        operand: PrimitiveOperand,
        default: ParameterDefault,
    },
    MissingParameter {
        operand: PrimitiveOperand,
    },
    DuplicateOperand {
        operand: PrimitiveOperand,
    },
    DuplicateSpelling {
        name: &'static str,
        profiles: PrimitiveProfiles,
    },
    DuplicateWebIdentity {
        command: &'static str,
        web_operand: Option<i64>,
        profiles: PrimitiveProfiles,
    },
    DuplicateParameterCell {
        cell: ParameterCell,
        profiles: PrimitiveProfiles,
    },
}

/// Borrowed canonical catalogue with exhaustive validation.
#[derive(Clone, Copy, Debug)]
pub struct PrimitiveCatalogue {
    descriptors: &'static [PrimitiveDescriptor],
}

impl PrimitiveCatalogue {
    #[must_use]
    pub const fn new(descriptors: &'static [PrimitiveDescriptor]) -> Self {
        Self { descriptors }
    }

    #[must_use]
    pub const fn descriptors(self) -> &'static [PrimitiveDescriptor] {
        self.descriptors
    }

    /// Returns every structural and uniqueness violation, rather than hiding
    /// later collisions behind the first error.
    #[must_use]
    pub fn validate(self) -> Vec<CatalogueValidationError> {
        let mut errors = Vec::new();
        let mut operands = HashMap::new();
        let mut spellings: HashMap<&str, Vec<PrimitiveProfiles>> = HashMap::new();
        let mut web_identities: HashMap<WebIdentityKey<'_>, Vec<WebIdentityOccurrence<'_>>> =
            HashMap::new();
        let mut parameter_cells: HashMap<ParameterCell, Vec<PrimitiveProfiles>> = HashMap::new();

        for descriptor in self.descriptors {
            validate_descriptor(descriptor, &mut errors);
            if operands.insert(descriptor.operand, ()).is_some() {
                errors.push(CatalogueValidationError::DuplicateOperand {
                    operand: descriptor.operand,
                });
            }

            for spelling in descriptor.spellings {
                for &profiles in spellings.entry(spelling.name).or_default().iter() {
                    let overlap = profiles_intersection(profiles, spelling.profiles);
                    if !overlap.is_empty() {
                        errors.push(CatalogueValidationError::DuplicateSpelling {
                            name: spelling.name,
                            profiles: overlap,
                        });
                    }
                }
                spellings
                    .entry(spelling.name)
                    .or_default()
                    .push(spelling.profiles);
            }

            for identity in descriptor.web {
                let identities = web_identities
                    .entry((identity.command, identity.operand))
                    .or_default();
                for &(profiles, collision_group) in identities.iter() {
                    let overlap = profiles_intersection(profiles, identity.profiles);
                    let explicitly_shared = identity.collision_group.is_some()
                        && identity.collision_group == collision_group;
                    if !overlap.is_empty() && !explicitly_shared {
                        errors.push(CatalogueValidationError::DuplicateWebIdentity {
                            command: identity.command,
                            web_operand: identity.operand,
                            profiles: overlap,
                        });
                    }
                }
                identities.push((identity.profiles, identity.collision_group));
            }

            if let Some((cell, _)) = descriptor.parameter {
                let cells = parameter_cells.entry(cell).or_default();
                for &profiles in cells.iter() {
                    let overlap = profiles_intersection(profiles, descriptor.profiles);
                    if !overlap.is_empty() {
                        errors.push(CatalogueValidationError::DuplicateParameterCell {
                            cell,
                            profiles: overlap,
                        });
                    }
                }
                cells.push(descriptor.profiles);
            }
        }
        errors
    }
}

fn validate_descriptor(
    descriptor: &PrimitiveDescriptor,
    errors: &mut Vec<CatalogueValidationError>,
) {
    if descriptor.profiles.is_empty() {
        errors.push(CatalogueValidationError::EmptyProfiles {
            operand: descriptor.operand,
        });
    }
    if !descriptor.spellings.is_empty()
        && !descriptor
            .spellings
            .iter()
            .any(|spelling| spelling.kind == SpellingKind::Canonical)
    {
        errors.push(CatalogueValidationError::MissingCanonicalSpelling {
            operand: descriptor.operand,
        });
    }
    for spelling in descriptor.spellings {
        if spelling.name.is_empty() {
            errors.push(CatalogueValidationError::EmptySpelling {
                operand: descriptor.operand,
            });
        }
        if spelling.profiles.is_empty() || !descriptor.profiles.contains_all(spelling.profiles) {
            errors.push(CatalogueValidationError::SpellingOutsideProfiles {
                operand: descriptor.operand,
                name: spelling.name,
            });
        }
    }
    for identity in descriptor.web {
        if identity.profiles.is_empty() || !descriptor.profiles.contains_all(identity.profiles) {
            errors.push(CatalogueValidationError::WebIdentityOutsideProfiles {
                operand: descriptor.operand,
                command: identity.command,
            });
        }
    }
    match descriptor.parameter {
        Some((cell, default)) => {
            if parameter_domain(cell.class) != descriptor.operand.domain
                || u64::from(cell.index) != descriptor.operand.value
            {
                errors.push(CatalogueValidationError::ParameterDomainMismatch {
                    operand: descriptor.operand,
                    cell,
                });
            }
            if !parameter_default_matches(cell.class, default) {
                errors.push(CatalogueValidationError::ParameterDefaultMismatch {
                    operand: descriptor.operand,
                    default,
                });
            }
        }
        None if is_parameter_domain(descriptor.operand.domain) => {
            errors.push(CatalogueValidationError::MissingParameter {
                operand: descriptor.operand,
            });
        }
        None => {}
    }
}

const fn parameter_default_matches(class: ParameterBankClass, default: ParameterDefault) -> bool {
    matches!(
        (class, default),
        (
            ParameterBankClass::Integer,
            ParameterDefault::Integer(_) | ParameterDefault::JobClock(_)
        ) | (ParameterBankClass::Dimension, ParameterDefault::Scaled(_))
            | (
                ParameterBankClass::Glue | ParameterBankClass::MathGlue,
                ParameterDefault::Glue(_)
            )
            | (ParameterBankClass::Tokens, ParameterDefault::EmptyTokens)
    )
}

const fn parameter_domain(class: ParameterBankClass) -> PrimitiveOperandDomain {
    match class {
        ParameterBankClass::Integer => PrimitiveOperandDomain::IntegerParameter,
        ParameterBankClass::Dimension => PrimitiveOperandDomain::DimensionParameter,
        ParameterBankClass::Glue => PrimitiveOperandDomain::GlueParameter,
        ParameterBankClass::MathGlue => PrimitiveOperandDomain::MathGlueParameter,
        ParameterBankClass::Tokens => PrimitiveOperandDomain::TokenParameter,
    }
}

const fn is_parameter_domain(domain: PrimitiveOperandDomain) -> bool {
    matches!(
        domain,
        PrimitiveOperandDomain::IntegerParameter
            | PrimitiveOperandDomain::DimensionParameter
            | PrimitiveOperandDomain::GlueParameter
            | PrimitiveOperandDomain::MathGlueParameter
            | PrimitiveOperandDomain::TokenParameter
    )
}

const fn profiles_intersection(
    left: PrimitiveProfiles,
    right: PrimitiveProfiles,
) -> PrimitiveProfiles {
    PrimitiveProfiles(left.0 & right.0)
}

#[cfg(test)]
mod tests;
