use std::collections::HashSet;

use tex_state::meaning::{ExpandablePrimitive, UnexpandablePrimitive};

use super::*;

#[test]
fn enum_views_are_complete_unique_and_round_trip() {
    let views = enum_primitive_views().collect::<Vec<_>>();
    assert_eq!(views.len(), 86 + 263);
    assert_eq!(
        views
            .iter()
            .map(|view| view.operand)
            .collect::<HashSet<_>>()
            .len(),
        views.len()
    );
    for view in views {
        assert_eq!(meaning_for_operand(view.operand), Some(view.meaning));
        assert!(!view.profiles.is_empty() || metadata_for(view.meaning).spellings.is_empty());
    }
}

#[test]
fn profile_registration_views_preserve_source_order_and_policy() {
    for profile in [
        PrimitiveProfile::Tex82,
        PrimitiveProfile::Etex26,
        PrimitiveProfile::LatexCompatibility,
        PrimitiveProfile::Pdftex14029,
    ] {
        let expected = metadata()
            .flat_map(|row| {
                row.spellings
                    .iter()
                    .filter(move |spelling| primitive_profile(spelling.set) == profile)
                    .map(move |spelling| (spelling.name, row.meaning))
            })
            .collect::<Vec<_>>();
        for policy in [
            InstallationPolicy::INITEX,
            InstallationPolicy::FORMAT_REGISTRY,
        ] {
            let actual = primitive_registrations(profile, policy)
                .map(|row| (row.name, row.meaning))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{profile:?} {policy:?}");
        }
    }
}

#[test]
fn pdftex_catalogue_exposes_the_enctex_mubyte_capability() {
    let registrations =
        primitive_registrations(PrimitiveProfile::Pdftex14029, InstallationPolicy::INITEX)
            .collect::<Vec<_>>();
    assert_eq!(
        registrations
            .iter()
            .find(|row| row.name == "mubyte")
            .map(|row| row.meaning),
        Some(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Mubyte
        ))
    );
    assert_eq!(
        primitive_observation_identity(
            CommandDialect::Pdftex14029,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mubyte),
        ),
        Some(("let", Some(10)))
    );
    assert!(
        primitive_registrations(PrimitiveProfile::Tex82, InstallationPolicy::INITEX)
            .all(|row| row.name != "mubyte")
    );
}

#[test]
fn observation_view_is_exhaustive_for_every_dialect() {
    for dialect in [
        CommandDialect::Tex82,
        CommandDialect::Etex26,
        CommandDialect::Pdftex14029,
    ] {
        for view in enum_primitive_views() {
            assert_eq!(
                primitive_observation_identity(dialect, view.meaning),
                Some(match view.meaning {
                    Meaning::ExpandablePrimitive(primitive) => {
                        super::super::metadata::expandable_identity(dialect, primitive)
                    }
                    Meaning::UnexpandablePrimitive(primitive) => {
                        super::super::metadata::unexpandable_identity(dialect, primitive)
                    }
                    _ => unreachable!(),
                })
            );
        }
    }
}

#[test]
fn documentation_rows_are_a_lossless_profile_projection() {
    for profile in [
        PrimitiveProfile::Tex82,
        PrimitiveProfile::Etex26,
        PrimitiveProfile::LatexCompatibility,
        PrimitiveProfile::Pdftex14029,
    ] {
        let registration = primitive_registrations(profile, InstallationPolicy::INITEX)
            .map(|row| (row.name, row.meaning))
            .collect::<Vec<_>>();
        let documentation = primitive_documentation_rows(profile).collect::<Vec<_>>();
        assert_eq!(documentation.len(), registration.len());
        for (documented, (name, meaning)) in documentation.iter().zip(registration) {
            assert_eq!(documented.name, name);
            assert_eq!(documented.operand, operand(meaning));
        }
    }
}

#[test]
fn markdown_documentation_table_is_generated_in_catalogue_order() {
    let table = render_primitive_documentation_table(PrimitiveProfile::LatexCompatibility);
    assert!(table.starts_with("| Primitive | Operand | Expansion | Prefix policy | Family |\n"));
    let rows = table.lines().skip(2).collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        primitive_documentation_rows(PrimitiveProfile::LatexCompatibility).count()
    );
    assert_eq!(
        rows.first().copied(),
        Some(
            "| `\\expanded` | `Expandable:53` | `Expandable` | `Forbidden` | `LatexCompatibility` |"
        )
    );
    assert_eq!(
        rows.last().copied(),
        Some(
            "| `\\ifincsname` | `Expandable:58` | `Expandable` | `Forbidden` | `LatexCompatibility` |"
        )
    );
}

#[test]
fn prefix_view_matches_the_existing_tex_web_partition() {
    for view in enum_primitive_views() {
        assert_eq!(
            view.prefix != PrefixAdmissibility::Forbidden,
            super::super::prefixed::is_prefixed_command(view.meaning),
            "{:?}",
            view.meaning
        );
    }
}

#[test]
fn enum_from_operand_maps_remain_complete() {
    for raw in 0..86 {
        let primitive = ExpandablePrimitive::from_operand(raw).expect("dense expandable operand");
        assert_eq!(
            meaning_for_operand(PrimitiveOperand::new(
                PrimitiveOperandDomain::Expandable,
                raw
            )),
            Some(Meaning::ExpandablePrimitive(primitive))
        );
    }
    let mut unexpandable_count = 0;
    for raw in 0..=266 {
        let Some(primitive) = UnexpandablePrimitive::from_operand(raw) else {
            continue;
        };
        unexpandable_count += 1;
        assert_eq!(
            meaning_for_operand(PrimitiveOperand::new(
                PrimitiveOperandDomain::Unexpandable,
                raw
            )),
            Some(Meaning::UnexpandablePrimitive(primitive))
        );
    }
    assert_eq!(unexpandable_count, 263);
}

#[test]
fn exceptional_catalogue_covers_frozen_private_and_profile_meanings() {
    let tex82 = special_primitive_views(PrimitiveProfile::Tex82).collect::<Vec<_>>();
    assert_eq!(
        tex82
            .iter()
            .find(|row| row.name == "nullfont")
            .expect("nullfont catalogue row")
            .meaning,
        Some(Meaning::Font(tex_state::font::NULL_FONT))
    );
    assert_eq!(
        tex82
            .iter()
            .find(|row| row.name == "endwrite")
            .expect("endwrite catalogue row")
            .meaning,
        None
    );
    for name in ["relax", "pagegoal", "deadcycles", "badness", "inputlineno"] {
        assert!(tex82.iter().any(|row| row.name == name), "{name}");
    }

    let pdftex = primitive_names(PrimitiveProfile::Pdftex14029);
    assert_eq!(pdftex.len(), 160);
    for name in [
        "partokencontext",
        "pdfoptionpdfminorversion",
        "pdfminorversion",
        "pdflastxform",
        "pdftexversion",
        "pdfpkmode",
        "mubyte",
    ] {
        assert!(pdftex.binary_search(&name).is_ok(), "{name}");
    }
}
