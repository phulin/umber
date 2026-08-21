use std::collections::HashSet;

use super::*;

#[test]
fn profile_parameter_views_have_exact_counts_and_order() {
    let tex82 = primitive_parameter_views(PrimitiveProfile::Tex82);
    assert_eq!(tex82.len(), 103);
    assert_eq!(tex82.first().map(|row| row.name), Some("pretolerance"));
    assert_eq!(tex82.get(54).map(|row| row.name), Some("fam"));
    assert_eq!(tex82.get(55).map(|row| row.name), Some("parindent"));
    assert_eq!(tex82.last().map(|row| row.name), Some("errhelp"));

    let etex = primitive_parameter_views(PrimitiveProfile::Etex26);
    assert_eq!(etex.len(), 12);
    assert_eq!(etex.first().map(|row| row.name), Some("everyeof"));
    assert_eq!(etex.last().map(|row| row.name), Some("synctex"));

    let pdftex = primitive_parameter_views(PrimitiveProfile::Pdftex14029);
    assert_eq!(pdftex.len(), 57);
    assert_eq!(pdftex.first().map(|row| row.name), Some("pdfoutput"));
    assert_eq!(
        pdftex.get(38).map(|row| row.name),
        Some("ignoreprimitiveerror")
    );
    assert_eq!(pdftex.get(39).map(|row| row.name), Some("partokencontext"));
    assert_eq!(pdftex.last().map(|row| row.name), Some("pdfpkmode"));
}

#[test]
fn web2c_synctex_parameter_is_extended_profile_only() {
    let etex = primitive_parameter_views(PrimitiveProfile::Etex26);
    let synctex = etex
        .iter()
        .find(|row| row.name == "synctex")
        .expect("pinned Web2C parameter");
    assert_eq!(
        synctex.cell,
        ParameterCell {
            class: ParameterBankClass::Integer,
            index: IntParam::SYNCTEX.raw(),
        }
    );
    assert_eq!(synctex.meaning, Meaning::IntParam(IntParam::SYNCTEX.raw()));
    assert_eq!(synctex.default, ParameterDefault::Integer(0));

    // The TeX82 oracle's change stack does not apply Web2C [54/SyncTeX].
    // Compatibility mode must therefore retain its exact primitive surface.
    assert!(
        primitive_parameter_views(PrimitiveProfile::Tex82)
            .iter()
            .all(|row| row.name != "synctex")
    );
}

#[test]
fn parameter_cells_are_unique_except_the_documented_pdf_minor_alias() {
    for profile in [PrimitiveProfile::Tex82, PrimitiveProfile::Etex26] {
        let rows = primitive_parameter_views(profile);
        assert_eq!(
            rows.iter()
                .map(|row| row.cell)
                .collect::<HashSet<_>>()
                .len(),
            rows.len()
        );
    }
    let pdftex = primitive_parameter_views(PrimitiveProfile::Pdftex14029);
    assert_eq!(
        pdftex
            .iter()
            .map(|row| row.cell)
            .collect::<HashSet<_>>()
            .len(),
        pdftex.len() - 1
    );
    let aliases = pdftex
        .iter()
        .filter(|row| row.cell.index == IntParam::PDF_MINOR_VERSION.raw())
        .map(|row| row.name)
        .collect::<Vec<_>>();
    assert_eq!(aliases, ["pdfoptionpdfminorversion", "pdfminorversion"]);
}

#[test]
fn defaults_preserve_job_clock_and_pdftex_nonzero_values() {
    let tex82 = primitive_parameter_views(PrimitiveProfile::Tex82);
    for (name, expected) in [
        ("tolerance", ParameterDefault::Integer(10_000)),
        ("mag", ParameterDefault::Integer(1_000)),
        ("escapechar", ParameterDefault::Integer(i32::from(b'\\'))),
        ("endlinechar", ParameterDefault::Integer(i32::from(b'\r'))),
        ("newlinechar", ParameterDefault::Integer(0)),
        ("maxdeadcycles", ParameterDefault::Integer(25)),
        ("hangafter", ParameterDefault::Integer(1)),
    ] {
        assert_eq!(
            tex82
                .iter()
                .find(|row| row.name == name)
                .map(|row| row.default),
            Some(expected),
            "tex.web §240 {name}"
        );
    }
    assert_eq!(
        tex82
            .iter()
            .find(|row| row.name == "time")
            .map(|row| row.default),
        Some(ParameterDefault::JobClock(
            JobClockField::MinutesSinceMidnight
        ))
    );
    let pdftex = primitive_parameter_views(PrimitiveProfile::Pdftex14029);
    for (name, expected) in [
        ("partokencontext", ParameterDefault::Integer(0)),
        ("pdfcompresslevel", ParameterDefault::Integer(9)),
        ("pdfminorversion", ParameterDefault::Integer(4)),
        ("pdfhorigin", ParameterDefault::Scaled(4_736_287)),
        ("pdffirstlineheight", ParameterDefault::Scaled(-65_536_000)),
        ("pdfpxdimen", ParameterDefault::Scaled(65_782)),
        ("pdfpagesattr", ParameterDefault::EmptyTokens),
    ] {
        assert_eq!(
            pdftex
                .iter()
                .find(|row| row.name == name)
                .map(|row| row.default),
            Some(expected),
            "{name}"
        );
    }
}

#[test]
fn fresh_default_batches_have_one_row_per_physical_cell() {
    assert_eq!(
        fresh_parameter_defaults(PrimitiveProfile::Tex82).len(),
        99,
        "103 TeX82 rows minus four volatile clock cells"
    );
    assert_eq!(
        fresh_parameter_defaults(PrimitiveProfile::Etex26).len(),
        13,
        "twelve named parameters plus the extended-mode activation cell"
    );
    assert_eq!(
        fresh_parameter_defaults(PrimitiveProfile::Pdftex14029).len(),
        56,
        "the two minor-version spellings share one dense cell"
    );
}

#[test]
fn pdftex_defaults_cover_every_canonical_nonzero_and_zero_family() {
    for row in primitive_parameter_views(PrimitiveProfile::Pdftex14029) {
        let expected = match row.name {
            "pdfcompresslevel" => ParameterDefault::Integer(9),
            "pdfdecimaldigits" => ParameterDefault::Integer(3),
            "pdfimageresolution" => ParameterDefault::Integer(72),
            "pdfoptionpdfminorversion" | "pdfminorversion" => ParameterDefault::Integer(4),
            "pdfmajorversion" => ParameterDefault::Integer(1),
            "pdfgamma" => ParameterDefault::Integer(1_000),
            "pdfimagegamma" => ParameterDefault::Integer(2_200),
            "pdfimagehicolor" => ParameterDefault::Integer(1),
            "pdfhorigin" | "pdfvorigin" => ParameterDefault::Scaled(4_736_287),
            "pdffirstlineheight" | "pdflastlinedepth" | "pdfeachlineheight"
            | "pdfeachlinedepth" | "pdfignoreddimen" => ParameterDefault::Scaled(-65_536_000),
            "pdfpxdimen" => ParameterDefault::Scaled(65_782),
            _ => match row.cell.class {
                ParameterBankClass::Integer => ParameterDefault::Integer(0),
                ParameterBankClass::Dimension => ParameterDefault::Scaled(0),
                ParameterBankClass::Tokens => ParameterDefault::EmptyTokens,
                ParameterBankClass::Glue | ParameterBankClass::MathGlue => {
                    panic!("pdfTeX has no glue parameter rows")
                }
            },
        };
        assert_eq!(row.default, expected, "pdftex.web §§672/1064 {}", row.name);
    }
}

#[test]
fn parameter_meanings_match_cells_and_defaults_match_bank_classes() {
    for profile in [
        PrimitiveProfile::Tex82,
        PrimitiveProfile::Etex26,
        PrimitiveProfile::Pdftex14029,
    ] {
        for row in primitive_parameter_views(profile) {
            let expected = match row.cell.class {
                ParameterBankClass::Integer => Meaning::IntParam(row.cell.index),
                ParameterBankClass::Dimension => Meaning::DimenParam(row.cell.index),
                ParameterBankClass::Glue => Meaning::GlueParam(row.cell.index),
                ParameterBankClass::MathGlue => Meaning::MuGlueParam(row.cell.index),
                ParameterBankClass::Tokens => Meaning::TokParam(row.cell.index),
            };
            assert_eq!(row.meaning, expected, "{}", row.name);
            assert!(row.installation.contains(InstallationPolicy::INITEX));
            assert!(
                row.installation
                    .contains(InstallationPolicy::FORMAT_REGISTRY)
            );
        }
    }
}
