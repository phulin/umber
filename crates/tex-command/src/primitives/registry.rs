use tex_state::Universe;
use tex_state::meaning::Meaning;

#[cfg(test)]
use super::metadata::{
    EXPANDABLE_PRIMITIVES, PrimitiveMetadata, PrimitiveSet, UNEXPANDABLE_PRIMITIVES,
};
use super::{ExpansionClass, InstallationPolicy, PrimitiveProfile, primitive_registrations};

/// Installs TeX82's enum-backed unexpandable primitive meanings.
pub fn install_tex82_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Tex82,
        ExpansionClass::Unexpandable,
    );
}

/// Registers TeX82's enum-backed primitive meanings without shadowing a format.
pub fn register_tex82_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Tex82,
        ExpansionClass::Unexpandable,
    );
}

/// Installs e-TeX's enum-backed unexpandable primitive meanings.
pub fn install_etex_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Etex26,
        ExpansionClass::Unexpandable,
    );
}

/// Registers e-TeX's enum-backed primitive meanings without shadowing a format.
pub fn register_etex_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Etex26,
        ExpansionClass::Unexpandable,
    );
}

/// Installs pdfTeX's enum-backed unexpandable primitive meanings.
pub fn install_pdftex_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Unexpandable,
    );
}

/// Registers pdfTeX's enum-backed primitive meanings without shadowing a format.
pub fn register_pdftex_unexpandable_primitives(universe: &mut Universe) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Unexpandable,
    );
}

/// Installs TeX82's expandable primitive meanings for a fresh INITEX state.
pub fn install_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, true);
}

/// Reconstructs TeX82's immutable primitive lookup table after format load.
pub fn register_tex82_expandable_primitives(universe: &mut Universe) {
    configure_tex82_expandable_primitives(universe, false);
}

fn configure_tex82_expandable_primitives(universe: &mut Universe, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Tex82,
        ExpansionClass::Expandable,
    );
}

/// Installs e-TeX 2.6's expandable primitive meanings for a fresh INITEX state.
pub fn install_etex_expandable_primitives(universe: &mut Universe) {
    universe.set_int_param_global(tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE, 1);
    configure_etex_expandable_primitives(universe, true);
}

/// Reconstructs e-TeX 2.6's immutable primitive lookup table after format load.
pub fn register_etex_expandable_primitives(universe: &mut Universe) {
    configure_etex_expandable_primitives(universe, false);
}

fn configure_etex_expandable_primitives(universe: &mut Universe, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Etex26,
        ExpansionClass::Expandable,
    );
    for (name, value) in [
        (
            "eTeXversion",
            tex_state::meaning::InternalInteger::ETeXVersion,
        ),
        (
            "currentgrouplevel",
            tex_state::meaning::InternalInteger::CurrentGroupLevel,
        ),
        (
            "currentgrouptype",
            tex_state::meaning::InternalInteger::CurrentGroupType,
        ),
        (
            "currentiflevel",
            tex_state::meaning::InternalInteger::CurrentIfLevel,
        ),
        (
            "currentiftype",
            tex_state::meaning::InternalInteger::CurrentIfType,
        ),
        (
            "currentifbranch",
            tex_state::meaning::InternalInteger::CurrentIfBranch,
        ),
        (
            "lastnodetype",
            tex_state::meaning::InternalInteger::LastNodeType,
        ),
    ] {
        configure_primitive(universe, install, name, Meaning::InternalInteger(value));
    }
}

/// Installs expandable primitives required by Umber's supported LaTeX
/// compatibility profile but not provided by e-TeX 2.6 itself.
pub fn install_latex_expandable_primitives(universe: &mut Universe) {
    configure_latex_expandable_primitives(universe, true);
}

/// Reconstructs the LaTeX compatibility primitive table after format load.
pub fn register_latex_expandable_primitives(universe: &mut Universe) {
    configure_latex_expandable_primitives(universe, false);
}

fn configure_latex_expandable_primitives(universe: &mut Universe, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::LatexCompatibility,
        ExpansionClass::Expandable,
    );
}

/// Installs pdfTeX 1.40.29's implemented expandable identity surface.
pub fn install_pdftex_expandable_primitives(universe: &mut Universe) {
    configure_pdftex_expandable_primitives(universe, true);
}

/// Reconstructs pdfTeX 1.40.29's expandable primitive lookup table after a format load.
pub fn register_pdftex_expandable_primitives(universe: &mut Universe) {
    configure_pdftex_expandable_primitives(universe, false);
}

fn configure_pdftex_expandable_primitives(universe: &mut Universe, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Expandable,
    );
    for (name, value) in [
        (
            "pdftexversion",
            tex_state::meaning::InternalInteger::PdfTeXVersion,
        ),
        (
            "pdflastobj",
            tex_state::meaning::InternalInteger::PdfLastObject,
        ),
        (
            "pdflastxform",
            tex_state::meaning::InternalInteger::PdfLastXForm,
        ),
    ] {
        configure_primitive(universe, install, name, Meaning::InternalInteger(value));
    }
}

fn configure_primitive(universe: &mut Universe, install: bool, name: &str, meaning: Meaning) {
    universe.register_primitive_meaning(name, meaning);
    if install {
        let symbol = universe.intern(name);
        universe.set_meaning(symbol, meaning);
    }
}

fn configure_generated(
    universe: &mut Universe,
    install: bool,
    profile: PrimitiveProfile,
    expansion: ExpansionClass,
) {
    let policy = if install {
        InstallationPolicy::INITEX
    } else {
        InstallationPolicy::FORMAT_REGISTRY
    };
    for registration in primitive_registrations(profile, policy).filter(|registration| {
        matches!(
            (registration.meaning, expansion),
            (Meaning::ExpandablePrimitive(_), ExpansionClass::Expandable)
                | (
                    Meaning::UnexpandablePrimitive(_),
                    ExpansionClass::Unexpandable
                )
        )
    }) {
        configure_primitive(universe, install, registration.name, registration.meaning);
    }
}

#[cfg(test)]
fn configure_metadata(
    universe: &mut Universe,
    install: bool,
    set: PrimitiveSet,
    metadata: &[PrimitiveMetadata],
) {
    for primitive in metadata {
        for spelling in primitive.spellings.iter().filter(|spelling| {
            spelling.set == set
                && if install {
                    spelling.install_in_initex
                } else {
                    spelling.register_after_format_load
                }
        }) {
            configure_primitive(universe, install, spelling.name, primitive.meaning);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn metadata() -> impl Iterator<Item = &'static PrimitiveMetadata> {
        EXPANDABLE_PRIMITIVES.iter().chain(UNEXPANDABLE_PRIMITIVES)
    }

    fn cases(set: PrimitiveSet) -> Vec<(&'static str, Meaning)> {
        metadata()
            .flat_map(|primitive| {
                primitive
                    .spellings
                    .iter()
                    .filter(move |spelling| spelling.set == set)
                    .map(|spelling| (spelling.name, primitive.meaning))
            })
            .collect()
    }

    #[test]
    fn metadata_is_unique_and_represents_shared_meanings() {
        let mut meanings = Vec::new();
        for primitive in metadata() {
            assert!(
                !meanings.contains(&primitive.meaning),
                "duplicate metadata meaning"
            );
            meanings.push(primitive.meaning);
            let mut spellings = HashSet::new();
            assert!(
                primitive
                    .spellings
                    .iter()
                    .all(|spelling| spellings.insert((spelling.set, spelling.name))),
                "duplicate spelling on {:?}",
                primitive.meaning
            );
        }

        let expanded = EXPANDABLE_PRIMITIVES
            .iter()
            .find(|primitive| {
                primitive.meaning
                    == Meaning::ExpandablePrimitive(
                        tex_state::meaning::ExpandablePrimitive::Expanded,
                    )
            })
            .expect("expanded metadata");
        assert_eq!(
            expanded.spellings,
            &[
                super::super::metadata::PrimitiveSpelling {
                    set: PrimitiveSet::Latex,
                    name: "expanded",
                    install_in_initex: true,
                    register_after_format_load: true,
                },
                super::super::metadata::PrimitiveSpelling {
                    set: PrimitiveSet::Pdftex,
                    name: "expanded",
                    install_in_initex: true,
                    register_after_format_load: true,
                },
            ]
        );
    }

    #[test]
    fn installation_and_format_registration_consume_the_same_metadata() {
        for set in [
            PrimitiveSet::Tex82,
            PrimitiveSet::Etex,
            PrimitiveSet::Latex,
            PrimitiveSet::Pdftex,
        ] {
            let cases = cases(set);
            let mut fresh = Universe::new_with_plain_catcodes();
            configure_metadata(&mut fresh, true, set, EXPANDABLE_PRIMITIVES);
            configure_metadata(&mut fresh, true, set, UNEXPANDABLE_PRIMITIVES);
            for &(name, meaning) in &cases {
                let symbol = fresh.symbol(name).expect("installed spelling");
                assert_eq!(fresh.meaning(symbol), meaning, "fresh \\{name}");
                assert_eq!(fresh.primitive_meaning(name), Some(meaning));
            }

            let mut loaded = Universe::new_with_plain_catcodes();
            for &(name, _) in &cases {
                let symbol = loaded.intern(name);
                loaded.set_meaning(symbol, Meaning::Relax);
            }
            configure_metadata(&mut loaded, false, set, EXPANDABLE_PRIMITIVES);
            configure_metadata(&mut loaded, false, set, UNEXPANDABLE_PRIMITIVES);
            for &(name, meaning) in &cases {
                let symbol = loaded.symbol(name).expect("shadowed spelling");
                assert_eq!(loaded.meaning(symbol), Meaning::Relax, "format \\{name}");
                assert_eq!(loaded.primitive_meaning(name), Some(meaning));
            }
        }
    }

    #[test]
    fn generated_install_and_format_loops_are_byte_value_and_order_identical() {
        for (set, profile) in [
            (PrimitiveSet::Tex82, PrimitiveProfile::Tex82),
            (PrimitiveSet::Etex, PrimitiveProfile::Etex26),
            (PrimitiveSet::Latex, PrimitiveProfile::LatexCompatibility),
            (PrimitiveSet::Pdftex, PrimitiveProfile::Pdftex14029),
        ] {
            for (metadata, expansion) in [
                (EXPANDABLE_PRIMITIVES, ExpansionClass::Expandable),
                (UNEXPANDABLE_PRIMITIVES, ExpansionClass::Unexpandable),
            ] {
                for install in [true, false] {
                    let mut predecessor = Universe::new_with_plain_catcodes();
                    let mut generated = Universe::new_with_plain_catcodes();
                    if !install {
                        for (name, _) in cases(set) {
                            let old = predecessor.intern(name);
                            predecessor.set_meaning(old, Meaning::Relax);
                            let new = generated.intern(name);
                            generated.set_meaning(new, Meaning::Relax);
                        }
                    }
                    configure_metadata(&mut predecessor, install, set, metadata);
                    configure_generated(&mut generated, install, profile, expansion);
                    for (name, meaning) in cases(set).into_iter().filter(|(_, meaning)| {
                        matches!(
                            (meaning, expansion),
                            (Meaning::ExpandablePrimitive(_), ExpansionClass::Expandable)
                                | (
                                    Meaning::UnexpandablePrimitive(_),
                                    ExpansionClass::Unexpandable
                                )
                        )
                    }) {
                        assert_eq!(generated.primitive_meaning(name), Some(meaning));
                        assert_eq!(
                            generated.primitive_token(name),
                            predecessor.primitive_token(name)
                        );
                        let generated_symbol = generated.symbol(name).expect("generated symbol");
                        let predecessor_symbol =
                            predecessor.symbol(name).expect("predecessor symbol");
                        assert_eq!(
                            generated.meaning(generated_symbol),
                            predecessor.meaning(predecessor_symbol),
                            "{profile:?} {expansion:?} install={install} \\{name}"
                        );
                    }
                }
            }
        }
    }
}
