use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::meaning::MeaningFlags;

#[cfg(test)]
use super::metadata::{
    EXPANDABLE_PRIMITIVES, PrimitiveMetadata, PrimitiveSet, UNEXPANDABLE_PRIMITIVES,
};
use super::{
    ExpansionClass, InstallationPolicy, PrimitiveProfile, primitive_parameter_views,
    primitive_registrations, special_primitive_views,
};

/// Installs TeX82's enum-backed unexpandable primitive meanings.
pub fn install_tex82_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Tex82,
        ExpansionClass::Unexpandable,
    );
    configure_specials(universe, true, PrimitiveProfile::Tex82, |name| {
        matches!(name, "relax" | "nullfont")
    });
    configure_parameters(universe, true, PrimitiveProfile::Tex82);
    configure_specials(universe, true, PrimitiveProfile::Tex82, |name| {
        !matches!(name, "relax" | "nullfont")
    });
}

/// Registers TeX82's enum-backed primitive meanings without shadowing a format.
pub fn register_tex82_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Tex82,
        ExpansionClass::Unexpandable,
    );
    configure_specials(universe, false, PrimitiveProfile::Tex82, |name| {
        matches!(name, "relax" | "nullfont")
    });
    configure_parameters(universe, false, PrimitiveProfile::Tex82);
    configure_specials(universe, false, PrimitiveProfile::Tex82, |name| {
        !matches!(name, "relax" | "nullfont")
    });
}

/// Installs e-TeX's enum-backed unexpandable primitive meanings.
pub fn install_etex_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Etex26,
        ExpansionClass::Unexpandable,
    );
    configure_parameters(universe, true, PrimitiveProfile::Etex26);
}

/// Registers e-TeX's enum-backed primitive meanings without shadowing a format.
pub fn register_etex_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Etex26,
        ExpansionClass::Unexpandable,
    );
    configure_parameters(universe, false, PrimitiveProfile::Etex26);
}

/// Installs pdfTeX's enum-backed unexpandable primitive meanings.
pub fn install_pdftex_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        true,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Unexpandable,
    );
    configure_parameters(universe, true, PrimitiveProfile::Pdftex14029);
    configure_specials(universe, true, PrimitiveProfile::Pdftex14029, |name| {
        !matches!(name, "pdftexversion" | "pdflastobj")
    });
}

/// Registers pdfTeX's enum-backed primitive meanings without shadowing a format.
pub fn register_pdftex_unexpandable_primitives<G>(universe: &mut Universe<G>) {
    configure_generated(
        universe,
        false,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Unexpandable,
    );
    configure_parameters(universe, false, PrimitiveProfile::Pdftex14029);
    configure_specials(universe, false, PrimitiveProfile::Pdftex14029, |name| {
        !matches!(name, "pdftexversion" | "pdflastobj")
    });
}

/// Installs TeX82's expandable primitive meanings for a fresh INITEX state.
pub fn install_tex82_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_tex82_expandable_primitives(universe, true);
}

/// Reconstructs TeX82's immutable primitive lookup table after format load.
pub fn register_tex82_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_tex82_expandable_primitives(universe, false);
}

fn configure_tex82_expandable_primitives<G>(universe: &mut Universe<G>, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Tex82,
        ExpansionClass::Expandable,
    );
}

/// Installs e-TeX 2.6's expandable primitive meanings for a fresh INITEX state.
pub fn install_etex_expandable_primitives<G>(universe: &mut Universe<G>) {
    universe
        .assign_int_param(
            tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("e-TeX mode parameter is admitted");
    configure_etex_expandable_primitives(universe, true);
}

/// Reconstructs e-TeX 2.6's immutable primitive lookup table after format load.
pub fn register_etex_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_etex_expandable_primitives(universe, false);
}

fn configure_etex_expandable_primitives<G>(universe: &mut Universe<G>, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Etex26,
        ExpansionClass::Expandable,
    );
    configure_specials(universe, install, PrimitiveProfile::Etex26, |_| true);
}

/// Installs expandable primitives required by Umber's supported LaTeX
/// compatibility profile but not provided by e-TeX 2.6 itself.
pub fn install_latex_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_latex_expandable_primitives(universe, true);
}

/// Reconstructs the LaTeX compatibility primitive table after format load.
pub fn register_latex_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_latex_expandable_primitives(universe, false);
}

fn configure_latex_expandable_primitives<G>(universe: &mut Universe<G>, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::LatexCompatibility,
        ExpansionClass::Expandable,
    );
}

/// Installs pdfTeX 1.40.29's implemented expandable identity surface.
pub fn install_pdftex_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_pdftex_expandable_primitives(universe, true);
}

/// Reconstructs pdfTeX 1.40.29's expandable primitive lookup table after a format load.
pub fn register_pdftex_expandable_primitives<G>(universe: &mut Universe<G>) {
    configure_pdftex_expandable_primitives(universe, false);
}

fn configure_pdftex_expandable_primitives<G>(universe: &mut Universe<G>, install: bool) {
    configure_generated(
        universe,
        install,
        PrimitiveProfile::Pdftex14029,
        ExpansionClass::Expandable,
    );
    configure_specials(universe, install, PrimitiveProfile::Pdftex14029, |name| {
        matches!(name, "pdftexversion" | "pdflastobj" | "pdflastxform")
    });
}

fn configure_parameters<G>(universe: &mut Universe<G>, install: bool, profile: PrimitiveProfile) {
    for row in primitive_parameter_views(profile) {
        configure_primitive(universe, install, row.name, row.meaning);
    }
}

fn configure_specials<G>(
    universe: &mut Universe<G>,
    install: bool,
    profile: PrimitiveProfile,
    include: impl Fn(&str) -> bool,
) {
    for row in special_primitive_views(profile).filter(|row| include(row.name)) {
        match (row.name, row.meaning) {
            ("nullfont", Some(meaning)) => configure_nullfont(universe, install, meaning),
            ("endwrite", None) => configure_write_stopper(universe),
            (_, Some(meaning)) => configure_primitive(universe, install, row.name, meaning),
            (_, None) => unreachable!("unknown store-local catalogue meaning"),
        }
    }
}

fn configure_nullfont<G>(universe: &mut Universe<G>, install: bool, meaning: Meaning) {
    // TeX82 §§259/1334 count the ordinary primitive's hash entry even though
    // §222 also gives `frozen_null_font` a fixed alias with the same spelling.
    universe.register_primitive_meaning("nullfont", meaning);
    if install {
        universe.install_primitive_meaning("nullfont", meaning);
    }
}

/// Registers TeX82's inaccessible outer `\endwrite` sentinel. Its macro
/// handles are store-local, so the catalogue owns the spelling and policy
/// while this installation seam constructs the meaning in the target store.
fn configure_write_stopper<G>(universe: &mut Universe<G>) {
    if universe.primitive_token("endwrite").is_some() {
        return;
    }
    let definition = universe
        .allocate_definition(&[], &[])
        .expect("frozen write stopper allocation");
    universe.register_primitive_word(
        "endwrite",
        tex_state::MeaningWord::macro_definition(MeaningFlags::OUTER, definition),
    );
}

fn configure_primitive<G>(universe: &mut Universe<G>, install: bool, name: &str, meaning: Meaning) {
    if install {
        universe.install_primitive_meaning(name, meaning);
    } else {
        universe.register_primitive_meaning(name, meaning);
    }
}

fn configure_generated<G>(
    universe: &mut Universe<G>,
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
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn metadata() -> impl Iterator<Item = &'static PrimitiveMetadata> {
        EXPANDABLE_PRIMITIVES.iter().chain(UNEXPANDABLE_PRIMITIVES)
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
    fn web2c_synctex_parameter_survives_extended_format_round_trip() {
        let mut compatibility = Universe::new();
        install_tex82_expandable_primitives(&mut compatibility);
        install_tex82_unexpandable_primitives(&mut compatibility);
        assert_eq!(compatibility.symbol("synctex"), None);

        let mut extended = Universe::new();
        install_tex82_expandable_primitives(&mut extended);
        install_tex82_unexpandable_primitives(&mut extended);
        install_etex_expandable_primitives(&mut extended);
        install_etex_unexpandable_primitives(&mut extended);
        let synctex = extended.symbol("synctex").expect("Web2C parameter symbol");
        assert_eq!(
            extended.meaning(synctex),
            Meaning::IntParam(tex_state::env::banks::IntParam::SYNCTEX.raw())
        );
        extended.set_int_param_global(tex_state::env::banks::IntParam::SYNCTEX, 7);
        let count = extended.engine_usage_statistics().control_sequences;

        let image = extended.dump_format().expect("extended format dumps");
        let mut loaded = Universe::from_format(tex_state::World::default(), &image)
            .expect("extended format reloads");
        let loaded_synctex = loaded.symbol("synctex").expect("restored parameter symbol");
        assert_eq!(loaded.meaning(loaded_synctex), extended.meaning(synctex));
        assert_eq!(
            loaded.int_param(tex_state::env::banks::IntParam::SYNCTEX),
            7
        );
        assert_eq!(loaded.engine_usage_statistics().control_sequences, count);
    }

    #[test]
    fn etex_initex_profile_includes_the_merged_web_string_pool() {
        fn install_tex82(universe: &mut Universe) {
            install_tex82_expandable_primitives(universe);
            install_tex82_unexpandable_primitives(universe);
        }

        fn loaded_capacity(universe: &Universe) -> (usize, usize) {
            let image = universe.dump_format().expect("primitive state dumps");
            let mut loaded = Universe::from_format(tex_state::World::default(), &image)
                .expect("primitive state reloads");
            let usage = loaded.engine_usage_statistics();
            (usage.string_capacity, usage.string_character_capacity)
        }

        let mut etex = Universe::new();
        let mut tex82 = Universe::new();
        install_tex82(&mut tex82);
        let tex82_capacity = loaded_capacity(&tex82);
        install_tex82(&mut etex);
        install_etex_expandable_primitives(&mut etex);
        install_etex_unexpandable_primitives(&mut etex);
        let once = loaded_capacity(&etex);

        // TeX82 §§47/50 and e-TeX [1.2] define the merged static pool and
        // primitive spellings. Pinned `init_prim` stops prove the e-TeX image
        // is exactly 119 strings and 1621 characters beyond TeX82; the typed
        // registry must preserve both physical coordinates.
        assert_eq!(tex82_capacity.0 - once.0, 119);
        assert_eq!(tex82_capacity.1 - once.1, 1_621);

        // Selecting the profile explicitly is a no-op because each installer
        // already selected TeX82 §§47/50's merged e-TeX WEB vocabulary.
        etex.select_string_pool_profile(tex_state::StringPoolProfile::Etex26);
        assert_eq!(loaded_capacity(&etex), once);

        // Both e-TeX installation halves select the profile, and repeated
        // setup must not charge TeX82 §§47/50's static pool twice.
        install_etex_expandable_primitives(&mut etex);
        install_etex_unexpandable_primitives(&mut etex);
        assert_eq!(loaded_capacity(&etex), once);
    }
}
