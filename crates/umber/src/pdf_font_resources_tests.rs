use super::*;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

fn used_unmapped_fonts(names: &[&str]) -> Universe {
    let mut stores = Universe::with_world(tex_state::World::memory());
    prepare_pdftex_run_stores(&mut stores);
    for (name, bytes) in [
        ("cmr10", CMR10 as &[u8]),
        ("cmsy10", CMSY10 as &[u8]),
        ("cmex10", CMEX10 as &[u8]),
    ] {
        stores
            .world_mut()
            .set_memory_file(format!("{name}.tfm"), bytes.to_vec())
            .expect("seed TFM");
    }
    let definitions = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let suffix = char::from(b'a' + u8::try_from(index).expect("short font fixture"));
            format!("\\font\\f{suffix}={name} \\pdfmapline{{-{name}}}")
        })
        .collect::<String>();
    let uses = names
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let suffix = char::from(b'a' + u8::try_from(index).expect("short font fixture"));
            format!("\\f{suffix} A")
        })
        .collect::<String>();
    run_memory_with_stores(
        &format!(
            concat!(
                "\\pdfoutput=1 ",
                "{definitions}",
                "\\shipout\\hbox{{{uses}}}\\end",
            ),
            definitions = definitions,
            uses = uses,
        ),
        &mut stores,
    )
    .expect("PDF run");
    stores
}

#[test]
fn virtual_font_names_never_reach_the_real_font_pk_fallback() {
    let mut ordinary = used_unmapped_fonts(&["cmr10"]);
    let mut requested = Vec::new();
    let error = provide_pdf_font_resources_at_dpi(&mut ordinary, 600, |_stores, name| {
        requested.push(name.to_vec());
        Err("stop after observing request".to_owned())
    })
    .expect_err("an ordinary unmapped font requests PK data");
    assert_eq!(error, "stop after observing request");
    assert_eq!(requested, [b"cmr10.600pk".to_vec()]);

    let mut virtual_roots = used_unmapped_fonts(&["cmr10", "cmsy10", "cmex10"]);
    let excluded = BTreeSet::from([b"cmr10".to_vec(), b"cmsy10".to_vec()]);
    let mut requested = Vec::new();
    let error = provide_pdf_font_resources_excluding_at_dpi(
        &mut virtual_roots,
        600,
        &excluded,
        |_stores, name| {
            requested.push(name.to_vec());
            Err("stop after observing request".to_owned())
        },
    )
    .expect_err("the remaining real font still uses PK fallback");
    assert_eq!(error, "stop after observing request");
    assert_eq!(requested, [b"cmex10.600pk".to_vec()]);
}
