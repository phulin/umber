use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::meaning::MeaningFlags;

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
