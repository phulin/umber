//! Generated, behavior-free views of the enum-backed primitive catalogue.
//!
//! The declarative rows live in `primitive_metadata.rs`. This module is the
//! only place that projects those rows into profile, installation, policy,
//! observation, and documentation views. Execution dispatch remains in the
//! processor and executor.

use tex_state::meaning::{InternalInteger, Meaning, UnexpandablePrimitive};
use tex_state::page::{PageDimension, PageInteger};

use super::catalogue::{
    DocumentationFamily, ExpansionClass, InstallationPolicy, PrefixAdmissibility, PrimitiveOperand,
    PrimitiveOperandDomain, PrimitiveProfile, PrimitiveProfiles, SpellingKind,
};
use super::metadata::{
    EXPANDABLE_PRIMITIVES, PrimitiveMetadata, PrimitiveSet, UNEXPANDABLE_PRIMITIVES,
};
use crate::CommandDialect;

/// One spelling projected from the canonical enum-backed catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveRegistration {
    pub name: &'static str,
    pub meaning: Meaning,
    pub profile: PrimitiveProfile,
    pub kind: SpellingKind,
    pub installation: InstallationPolicy,
}

/// One non-enum meaning projected from the canonical catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpecialPrimitiveView {
    pub name: &'static str,
    /// Store-independent meaning. `None` identifies the frozen `\endwrite`
    /// macro whose handles must be constructed in the destination store.
    pub meaning: Option<Meaning>,
}

/// Returns parameter and exceptional meanings in their canonical installation
/// order for one profile layer.
pub fn special_primitive_views(
    profile: PrimitiveProfile,
) -> impl Iterator<Item = SpecialPrimitiveView> {
    let rows: &'static [SpecialPrimitiveView] = match profile {
        PrimitiveProfile::Tex82 => TEX82_SPECIAL_PRIMITIVES,
        PrimitiveProfile::Etex26 => ETEX_SPECIAL_PRIMITIVES,
        PrimitiveProfile::LatexCompatibility => &[],
        PrimitiveProfile::Pdftex14029 => PDFTEX_SPECIAL_PRIMITIVES,
    };
    rows.iter().copied()
}

/// Returns the exact spelling inventory of one profile layer, derived from
/// enum, parameter, and exceptional catalogue views.
#[must_use]
pub fn primitive_names(profile: PrimitiveProfile) -> Vec<&'static str> {
    let mut names = primitive_registrations(profile, InstallationPolicy::INITEX)
        .map(|row| row.name)
        .chain(
            super::parameters::primitive_parameter_views(profile)
                .into_iter()
                .map(|row| row.name),
        )
        .chain(special_primitive_views(profile).map(|row| row.name))
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    names
}

/// One documentation row projected from the canonical catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveDocumentationRow {
    pub operand: PrimitiveOperand,
    pub name: &'static str,
    pub profile: PrimitiveProfile,
    pub expansion: ExpansionClass,
    pub prefix: PrefixAdmissibility,
    pub family: DocumentationFamily,
}

/// One enum-backed primitive projected without execution behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnumPrimitiveView {
    pub operand: PrimitiveOperand,
    pub meaning: Meaning,
    pub profiles: PrimitiveProfiles,
    pub expansion: ExpansionClass,
    pub prefix: PrefixAdmissibility,
}

/// Returns catalogue rows in their canonical declaration order.
pub fn enum_primitive_views() -> impl Iterator<Item = EnumPrimitiveView> {
    metadata().map(|row| EnumPrimitiveView {
        operand: operand(row.meaning),
        meaning: row.meaning,
        profiles: profiles(row),
        expansion: expansion(row.meaning),
        prefix: prefix(row.meaning),
    })
}

/// Returns the exact registration slice for one layer and installation mode.
///
/// Layer slices are deliberately not expanded through profile inheritance:
/// callers compose TeX82, e-TeX, LaTeX compatibility, and pdfTeX layers in
/// the same explicit order as the engine profile they are constructing.
pub fn primitive_registrations(
    profile: PrimitiveProfile,
    policy: InstallationPolicy,
) -> impl Iterator<Item = PrimitiveRegistration> {
    metadata().flat_map(move |row| {
        row.spellings
            .iter()
            .filter(move |spelling| {
                primitive_profile(spelling.set) == profile
                    && spelling.installation().contains(policy)
            })
            .map(move |spelling| PrimitiveRegistration {
                name: spelling.name,
                meaning: row.meaning,
                profile,
                kind: SpellingKind::Canonical,
                installation: spelling.installation(),
            })
    })
}

/// Returns the canonical observation identity for an enum-backed meaning.
pub fn primitive_observation_identity(
    dialect: CommandDialect,
    meaning: Meaning,
) -> Option<(&'static str, Option<i64>)> {
    match meaning {
        Meaning::ExpandablePrimitive(primitive) => {
            Some(super::metadata::expandable_identity(dialect, primitive))
        }
        Meaning::UnexpandablePrimitive(primitive) => {
            Some(super::metadata::unexpandable_identity(dialect, primitive))
        }
        _ => None,
    }
}

/// Returns stable documentation rows in catalogue and spelling order.
pub fn primitive_documentation_rows(
    profile: PrimitiveProfile,
) -> impl Iterator<Item = PrimitiveDocumentationRow> {
    enum_primitive_views().flat_map(move |view| {
        metadata_for(view.meaning)
            .spellings
            .iter()
            .filter(move |spelling| primitive_profile(spelling.set) == profile)
            .map(move |spelling| PrimitiveDocumentationRow {
                operand: view.operand,
                name: spelling.name,
                profile,
                expansion: view.expansion,
                prefix: view.prefix,
                family: documentation_family(spelling.set),
            })
    })
}

/// Renders a deterministic Markdown table directly from documentation rows.
#[must_use]
pub fn render_primitive_documentation_table(profile: PrimitiveProfile) -> String {
    let mut table = String::from(
        "| Primitive | Operand | Expansion | Prefix policy | Family |\n|---|---:|---|---|---|\n",
    );
    for row in primitive_documentation_rows(profile) {
        use std::fmt::Write as _;
        let _ = writeln!(
            table,
            "| `\\{}` | `{:?}:{}` | `{:?}` | `{:?}` | `{:?}` |",
            row.name, row.operand.domain, row.operand.value, row.expansion, row.prefix, row.family
        );
    }
    table
}

/// Stable operand-to-enum map generated from the catalogue.
pub fn meaning_for_operand(needle: PrimitiveOperand) -> Option<Meaning> {
    enum_primitive_views()
        .find(|view| view.operand == needle)
        .map(|view| view.meaning)
}

fn metadata() -> impl Iterator<Item = &'static PrimitiveMetadata> {
    EXPANDABLE_PRIMITIVES.iter().chain(UNEXPANDABLE_PRIMITIVES)
}

fn metadata_for(meaning: Meaning) -> &'static PrimitiveMetadata {
    metadata()
        .find(|row| row.meaning == meaning)
        .expect("generated enum view refers to a catalogue row")
}

const fn operand(meaning: Meaning) -> PrimitiveOperand {
    match meaning {
        Meaning::ExpandablePrimitive(primitive) => {
            PrimitiveOperand::new(PrimitiveOperandDomain::Expandable, primitive.operand())
        }
        Meaning::UnexpandablePrimitive(primitive) => {
            PrimitiveOperand::new(PrimitiveOperandDomain::Unexpandable, primitive.operand())
        }
        _ => panic!("enum catalogue contains a non-enum meaning"),
    }
}

const fn expansion(meaning: Meaning) -> ExpansionClass {
    match meaning {
        Meaning::ExpandablePrimitive(_) => ExpansionClass::Expandable,
        Meaning::UnexpandablePrimitive(_) => ExpansionClass::Unexpandable,
        _ => panic!("enum catalogue contains a non-enum meaning"),
    }
}

fn profiles(row: &PrimitiveMetadata) -> PrimitiveProfiles {
    row.spellings
        .iter()
        .fold(PrimitiveProfiles::NONE, |set, spelling| {
            set.union(PrimitiveProfiles::only(primitive_profile(spelling.set)))
        })
}

fn prefix(meaning: Meaning) -> PrefixAdmissibility {
    match meaning {
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected,
        ) => PrefixAdmissibility::Prefix,
        _ if super::prefixed::is_prefixed_command(meaning) => PrefixAdmissibility::Admissible,
        _ => PrefixAdmissibility::Forbidden,
    }
}

const fn primitive_profile(set: PrimitiveSet) -> PrimitiveProfile {
    match set {
        PrimitiveSet::Tex82 => PrimitiveProfile::Tex82,
        PrimitiveSet::Etex => PrimitiveProfile::Etex26,
        PrimitiveSet::Latex => PrimitiveProfile::LatexCompatibility,
        PrimitiveSet::Pdftex => PrimitiveProfile::Pdftex14029,
    }
}

const fn documentation_family(set: PrimitiveSet) -> DocumentationFamily {
    match set {
        PrimitiveSet::Tex82 => DocumentationFamily::Tex82,
        PrimitiveSet::Etex => DocumentationFamily::Etex26,
        PrimitiveSet::Latex => DocumentationFamily::LatexCompatibility,
        PrimitiveSet::Pdftex => DocumentationFamily::Pdftex14029,
    }
}

const TEX82_SPECIAL_PRIMITIVES: &[SpecialPrimitiveView] = &[
    SpecialPrimitiveView {
        name: "relax",
        meaning: Some(Meaning::Relax),
    },
    SpecialPrimitiveView {
        name: "nullfont",
        meaning: Some(Meaning::Font(tex_state::font::NULL_FONT)),
    },
    SpecialPrimitiveView {
        name: "pagegoal",
        meaning: Some(Meaning::PageDimension(PageDimension::Goal)),
    },
    SpecialPrimitiveView {
        name: "pagetotal",
        meaning: Some(Meaning::PageDimension(PageDimension::Total)),
    },
    SpecialPrimitiveView {
        name: "pagestretch",
        meaning: Some(Meaning::PageDimension(PageDimension::Stretch)),
    },
    SpecialPrimitiveView {
        name: "pagefilstretch",
        meaning: Some(Meaning::PageDimension(PageDimension::FilStretch)),
    },
    SpecialPrimitiveView {
        name: "pagefillstretch",
        meaning: Some(Meaning::PageDimension(PageDimension::FillStretch)),
    },
    SpecialPrimitiveView {
        name: "pagefilllstretch",
        meaning: Some(Meaning::PageDimension(PageDimension::FilllStretch)),
    },
    SpecialPrimitiveView {
        name: "pageshrink",
        meaning: Some(Meaning::PageDimension(PageDimension::Shrink)),
    },
    SpecialPrimitiveView {
        name: "pagedepth",
        meaning: Some(Meaning::PageDimension(PageDimension::Depth)),
    },
    SpecialPrimitiveView {
        name: "deadcycles",
        meaning: Some(Meaning::PageInteger(PageInteger::DeadCycles)),
    },
    SpecialPrimitiveView {
        name: "insertpenalties",
        meaning: Some(Meaning::PageInteger(PageInteger::InsertPenalties)),
    },
    SpecialPrimitiveView {
        name: "badness",
        meaning: Some(Meaning::InternalInteger(InternalInteger::Badness)),
    },
    SpecialPrimitiveView {
        name: "inputlineno",
        meaning: Some(Meaning::InternalInteger(InternalInteger::InputLineNumber)),
    },
    SpecialPrimitiveView {
        name: "endwrite",
        meaning: None,
    },
];

const ETEX_SPECIAL_PRIMITIVES: &[SpecialPrimitiveView] = &[
    SpecialPrimitiveView {
        name: "eTeXversion",
        meaning: Some(Meaning::InternalInteger(InternalInteger::ETeXVersion)),
    },
    SpecialPrimitiveView {
        name: "currentgrouplevel",
        meaning: Some(Meaning::InternalInteger(InternalInteger::CurrentGroupLevel)),
    },
    SpecialPrimitiveView {
        name: "currentgrouptype",
        meaning: Some(Meaning::InternalInteger(InternalInteger::CurrentGroupType)),
    },
    SpecialPrimitiveView {
        name: "currentiflevel",
        meaning: Some(Meaning::InternalInteger(InternalInteger::CurrentIfLevel)),
    },
    SpecialPrimitiveView {
        name: "currentiftype",
        meaning: Some(Meaning::InternalInteger(InternalInteger::CurrentIfType)),
    },
    SpecialPrimitiveView {
        name: "currentifbranch",
        meaning: Some(Meaning::InternalInteger(InternalInteger::CurrentIfBranch)),
    },
    SpecialPrimitiveView {
        name: "lastnodetype",
        meaning: Some(Meaning::InternalInteger(InternalInteger::LastNodeType)),
    },
];

const PDFTEX_SPECIAL_PRIMITIVES: &[SpecialPrimitiveView] = &[
    SpecialPrimitiveView {
        name: "pdfelapsedtime",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfElapsedTime)),
    },
    SpecialPrimitiveView {
        name: "pdfrandomseed",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfRandomSeed)),
    },
    SpecialPrimitiveView {
        name: "pdfshellescape",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfShellEscape)),
    },
    SpecialPrimitiveView {
        name: "pdflastannot",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastAnnot)),
    },
    SpecialPrimitiveView {
        name: "pdflastlink",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastLink)),
    },
    SpecialPrimitiveView {
        name: "pdflastxpos",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastXPos)),
    },
    SpecialPrimitiveView {
        name: "pdflastypos",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastYPos)),
    },
    SpecialPrimitiveView {
        name: "pdflastxform",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastXForm)),
    },
    SpecialPrimitiveView {
        name: "pdflastximage",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastXImage)),
    },
    SpecialPrimitiveView {
        name: "pdfretval",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfReturnValue)),
    },
    SpecialPrimitiveView {
        name: "pdflastximagepages",
        meaning: Some(Meaning::InternalInteger(
            InternalInteger::PdfLastXImagePages,
        )),
    },
    SpecialPrimitiveView {
        name: "pdflastximagecolordepth",
        meaning: Some(Meaning::InternalInteger(
            InternalInteger::PdfLastXImageColorDepth,
        )),
    },
    SpecialPrimitiveView {
        name: "pdftexversion",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfTeXVersion)),
    },
    SpecialPrimitiveView {
        name: "pdflastobj",
        meaning: Some(Meaning::InternalInteger(InternalInteger::PdfLastObject)),
    },
];

#[cfg(test)]
mod tests;
