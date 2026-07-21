use super::*;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");

fn used_unmapped_fonts(include_second: bool) -> Universe {
    let mut stores = Universe::with_world(tex_state::World::memory());
    prepare_pdftex_run_stores(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed TFM");
    stores
        .world_mut()
        .set_memory_file("cmsy10.tfm", CMSY10.to_vec())
        .expect("seed second TFM");
    let second = if include_second {
        "\\font\\g=cmsy10 \\pdfmapline{-cmsy10}"
    } else {
        ""
    };
    let second_use = if include_second { " \\g B" } else { "" };
    run_memory_with_stores(
        &format!(
            concat!(
                "\\pdfoutput=1 ",
                "\\font\\f=cmr10 ",
                "\\pdfmapline{{-cmr10}}",
                "{second}",
                "\\shipout\\hbox{{\\f A{second_use}}}\\end",
            ),
            second = second,
            second_use = second_use,
        ),
        &mut stores,
    )
    .expect("PDF run");
    stores
}

#[test]
fn virtual_font_names_never_reach_the_real_font_pk_fallback() {
    let mut ordinary = used_unmapped_fonts(false);
    let mut requested = Vec::new();
    let error = provide_pdf_font_resources_at_dpi(&mut ordinary, 600, |_stores, name| {
        requested.push(name.to_vec());
        Err("stop after observing request".to_owned())
    })
    .expect_err("an ordinary unmapped font requests PK data");
    assert_eq!(error, "stop after observing request");
    assert_eq!(requested, [b"cmr10.600pk".to_vec()]);

    let mut virtual_root = used_unmapped_fonts(true);
    let excluded = BTreeSet::from([b"cmr10".to_vec()]);
    let mut requested = Vec::new();
    let error = provide_pdf_font_resources_excluding_at_dpi(
        &mut virtual_root,
        600,
        &excluded,
        |_stores, name| {
            requested.push(name.to_vec());
            Err("stop after observing request".to_owned())
        },
    )
    .expect_err("the remaining real font still uses PK fallback");
    assert_eq!(error, "stop after observing request");
    assert_eq!(requested, [b"cmsy10.600pk".to_vec()]);
}
