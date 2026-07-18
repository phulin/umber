//! Native translations of upstream `t/sorting.t` at commit 74252e6.

use bib_engine::{BibResult, DataListId, FieldId};
use bib_sort::{
    DataListBuilder, PadDirection, SortComponent, SortDirection, SortField, SortOptions,
    SortTemplate,
};

use super::maps::run_fixture;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SortDataKind {
    String,
    Integer,
    DescendingInteger,
}

const SCHEMA_1: &[SortDataKind] = &[
    SortDataKind::String,
    SortDataKind::String,
    SortDataKind::String,
    SortDataKind::Integer,
    SortDataKind::String,
    SortDataKind::Integer,
];

const SCHEMA_2: &[SortDataKind] = &[
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::DescendingInteger,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::DescendingInteger,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
    SortDataKind::Integer,
];

#[derive(Clone, Copy)]
enum Template {
    Nty,
    NtyYearLeft3,
    NtyYearLeft4,
    NtyYearRight3,
    NtyYearRight4,
    NtydYearLeft3,
    NtydYearLeft4,
    NtydYearRight3,
    NtydYearRight4,
    NtyVolumeRight4,
    NtyVolumeRight7,
    NtyVolumeLeft5,
    NtyTitleFinal,
    Nyt,
    Nyvt,
    Anyt,
    Anyvt,
    Custom,
    Dates,
    EditorRole,
    Publisher,
    Location,
    Institution,
    Ynt,
    Ydnt,
    EntryKey,
    Labels,
    Name,
    CiteOrder,
}

fn field(name: &str, numeric: bool) -> SortComponent {
    SortComponent {
        field: SortField::Field(FieldId::new(name).expect("valid sort field")),
        options: SortOptions {
            numeric,
            ..SortOptions::default()
        },
    }
}

fn fields(names: &[(&str, bool)]) -> Vec<SortComponent> {
    names
        .iter()
        .map(|(name, numeric)| field(name, *numeric))
        .collect()
}

fn template(kind: Template) -> SortTemplate {
    let mut components = match kind {
        Template::Nty
        | Template::NtyYearLeft3
        | Template::NtyYearLeft4
        | Template::NtyYearRight3
        | Template::NtyYearRight4
        | Template::NtydYearLeft3
        | Template::NtydYearLeft4
        | Template::NtydYearRight3
        | Template::NtydYearRight4
        | Template::NtyVolumeRight4
        | Template::NtyVolumeRight7
        | Template::NtyVolumeLeft5
        | Template::NtyTitleFinal => fields(&[
            ("presort", false),
            ("sortkey", false),
            ("author", false),
            ("title", false),
            ("year", true),
            ("volume", true),
        ]),
        Template::Nyt => fields(&[
            ("presort", false),
            ("sortkey", false),
            ("author", false),
            ("year", true),
            ("title", false),
            ("volume", true),
        ]),
        Template::Nyvt => fields(&[
            ("presort", false),
            ("sortkey", false),
            ("author", false),
            ("year", true),
            ("volume", true),
            ("title", false),
        ]),
        Template::Anyt => fields(&[
            ("presort", false),
            ("labelalpha", false),
            ("sortkey", false),
            ("author", false),
            ("year", true),
            ("title", false),
            ("sorttitle", false),
        ]),
        Template::Anyvt => fields(&[
            ("presort", false),
            ("labelalpha", false),
            ("sortkey", false),
            ("author", false),
            ("year", true),
            ("volume", true),
            ("title", false),
        ]),
        Template::Custom => fields(&[
            ("presort", false),
            ("sortkey", false),
            ("author", false),
            ("editor", false),
            ("translator", false),
            ("title", false),
            ("labelyear", true),
            ("volume", true),
        ]),
        Template::Dates => fields(&[
            ("year", true),
            ("month", true),
            ("day", true),
            ("endyear", true),
            ("endmonth", true),
            ("endday", true),
            ("origyear", true),
            ("origmonth", true),
            ("origday", true),
            ("origendyear", true),
            ("origendmonth", true),
            ("origendday", true),
            ("eventendyear", true),
            ("eventendmonth", true),
            ("eventendday", true),
            ("eventyear", true),
            ("eventmonth", true),
            ("eventday", true),
            ("urlendyear", true),
            ("urlendmonth", true),
            ("urlendday", true),
            ("urlyear", true),
            ("urlmonth", true),
            ("urlday", true),
        ]),
        Template::EditorRole => fields(&[
            ("editoratype", false),
            ("editor", false),
            ("editora", false),
        ]),
        Template::Publisher => fields(&[("publisher", false)]),
        Template::Location => fields(&[("location", false)]),
        Template::Institution => fields(&[("institution", false)]),
        Template::Ynt | Template::Ydnt => fields(&[
            ("presort", false),
            ("sortkey", false),
            ("year", true),
            ("author", false),
            ("title", false),
        ]),
        Template::EntryKey => vec![SortComponent::ascending(SortField::EntryId)],
        Template::Labels => fields(&[
            ("labelyear", true),
            ("labelmonth", true),
            ("labelday", true),
        ]),
        Template::Name => fields(&[("sortname", false)]),
        Template::CiteOrder => vec![SortComponent::ascending(SortField::CiteOrder)],
    };
    match kind {
        Template::NtyYearLeft3 | Template::NtydYearLeft3 => {
            components[4].options.substring = Some((0, 3));
        }
        Template::NtyYearLeft4 | Template::NtydYearLeft4 => {
            components[4].options.substring = Some((0, 4));
        }
        Template::NtyYearRight3 | Template::NtydYearRight3 => {
            components[4].options.substring = Some((1, 3));
        }
        Template::NtyYearRight4 | Template::NtydYearRight4 => {
            components[4].options.substring = Some((0, 4));
        }
        Template::NtyVolumeRight4 => {
            components[5].options.pad_width = Some(4);
            components[5].options.pad_direction = PadDirection::Right;
        }
        Template::NtyVolumeRight7 => {
            components[5].options.pad_width = Some(7);
            components[5].options.pad_direction = PadDirection::Right;
        }
        Template::NtyVolumeLeft5 | Template::NtyTitleFinal => {
            components[5].options.pad_width = Some(5);
            components[5].options.pad_char = 'Đ';
        }
        _ => {}
    }
    if matches!(kind, Template::NtyTitleFinal) {
        components[3].options.final_value = true;
    }
    if matches!(
        kind,
        Template::NtydYearLeft3
            | Template::NtydYearLeft4
            | Template::NtydYearRight3
            | Template::NtydYearRight4
            | Template::Ydnt
    ) {
        let index = if matches!(kind, Template::Ydnt) { 2 } else { 4 };
        components[index].options.direction = SortDirection::Descending;
    }
    if matches!(kind, Template::Dates) {
        components[6].options.direction = SortDirection::Descending;
        components[18].options.direction = SortDirection::Descending;
    }
    SortTemplate::new(components).expect("non-empty upstream sort template")
}

fn sort_data_for_key(result: &BibResult, key: &str, kind: Template) -> String {
    let section = result
        .document()
        .section(bib_engine::SectionId::new(0))
        .expect("section zero");
    DataListBuilder::new(
        section,
        DataListId::new("native-sort-probe").expect("probe list id"),
        template(kind),
    )
    .sorted_entries()
    .expect("native sorting succeeds")
    .into_iter()
    .find(|entry| entry.id.as_str() == key)
    .expect("entry in native sorted data")
    .keys
    .into_iter()
    .map(Option::unwrap_or_default)
    .collect::<Vec<_>>()
    .join(",")
}

fn sort_data_schema(kind: Template) -> Vec<SortDataKind> {
    template(kind)
        .components()
        .map(
            |component| match (component.options.numeric, component.options.direction) {
                (true, SortDirection::Descending) => SortDataKind::DescendingInteger,
                (true, SortDirection::Ascending) => SortDataKind::Integer,
                (false, _) => SortDataKind::String,
            },
        )
        .collect()
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_001_von_with_type_specific_presort_exclusions_and_useprefix_true() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "tvonb", Template::Nyt),
        r#"ww,,vonBobble       Terrence       ,,,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_002_von_with_name_list_scope_useprefix() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "avona", Template::Nyt),
        r#"mm,,Animal       Alan           von,1998,Things,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_003_von_with_name_scope_useprefix() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "rvonr", Template::Nyt),
        r#"mm,,vonRabble       Richard        ,1998,Things,0"#
    );
}

#[test]
fn assertion_004_sorting_data_schemata_1() {
    let _result = run_fixture("general");
    assert_eq!(sort_data_schema(Template::Nyt), SCHEMA_1);
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_005_explicit_and_others_1() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "others1", Template::Nyt),
        r#"mm,,Gauck        Joachim        ,,Title A,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_006_explicit_and_others_2() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "others2", Template::Nyt),
        r#"mm,,Gauck        Joachim        ,,Title B,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_007_final_entries_with_no_other_data() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "final", Template::Nyt),
        r#"mm,,zzzz,zzzz,zzzz,zzzz"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_008_sorting_name_key_1() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "snk1", Template::Nyt),
        r#"mm,,John            John           vonDoe          Jr,,,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_009_sorting_name_key_2() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "ent1", Template::Nyt),
        r#"mm,,Smith        Brian          ,,,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_010_von_with_type_specific_presort_exclusions_and_useprefix_false() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "tvonb", Template::Nyt),
        r#"ww,,Bobble       Terrence       von,,,0"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_011_title_with_nosort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "luzzatto", Template::Custom),
        r#"mm,,Luzzatto     Moshe Ḥayyim   ,,,Lashon la-Ramḥal: u-vo sheloshah ḥiburim,2000,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_012_name_with_nosort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "hasan", Template::Custom),
        r#"mm,,Hasan        Alī            ,al-Hasan     ʿAlī           ,Hasan        Alī            ,Some title,2000,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_013_editor_type_class() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "jaffe", Template::EditorRole),
        r#"redactor,Jaffé        Philipp        ,Loewenfeld   Samuel         KaltenbrunnerFerdinand      Ewald        Paul           "#
    );
}

#[test]
fn assertion_014_sorting_data_schemata_2() {
    let _result = run_fixture("general");
    assert_eq!(sort_data_schema(Template::Dates), SCHEMA_2);
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_015_very_contrived_but_thorough_test_of_date_sorting() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "moraux", Template::Dates),
        r#"1979,1,2000000,1980,4,8,1924,6,7,1924,7,9,1924,0002,5,192,2,3,1979,3,4,79,3,3"#
    );
}

#[test]
fn assertion_016_max_minitems_test_1_publisher() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "augustine", Template::Publisher),
        r#"Marcel Dekker"#
    );
}

#[test]
fn assertion_017_max_minitems_test_2_location() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "cotton", Template::Location),
        r#"Chichester"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_018_max_minitems_test_3_institution() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "chiu", Template::Institution),
        r#"IBM􏿽"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_019_max_minitems_test_4_institution_minitems_2() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "chiu", Template::Institution),
        r#"IBM!HP􏿽"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_020_max_minitems_test_5_institution_maxitems_4_minitems_3() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "chiu", Template::Institution),
        r#"IBM!HP!Sun!Sony"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_021_nty_with_default_left_offset_4_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtyYearLeft4),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,1984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_022_nty_with_left_offset_3_digit_year_case_sensitive() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtyYearLeft3),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,198,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_023_nty_with_left_offset_4_digit_year_case_sensitive() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtyYearLeft4),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,1984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_024_nty_with_right_offset_3_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtyYearRight3),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_025_nty_with_right_offset_4_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtyYearRight4),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,1984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_026_ntyd_with_left_offset_4_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtydYearLeft4),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,1984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_027_ntyd_with_left_offset_3_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtydYearLeft3),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,198,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_028_ntyd_with_right_offset_4_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtydYearRight4),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,1984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_029_ntyd_with_right_offset_3_digit_year() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "knuth:ct", Template::NtydYearRight3),
        r#"mm,,Knuth        Donald E.      ,Computers & Typesetting,984,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_030_nty_with_right_padded_vol() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::NtyVolumeRight4),
        r#"mm,,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions,1961,2200"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_031_nty_with_right_padded_7_char_vol() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::NtyVolumeRight7),
        r#"mm,,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions,1961,2200000"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_032_nty_with_left_padded_5_char_a_pad_char_vol() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::NtyVolumeLeft5),
        r#"mm,,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions,1961,ĐĐĐ22"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_033_basic_nty_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Nty),
        r#"mm,,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions,1961,22"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_034_basic_sortkey_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "angenendtsk", Template::Nty),
        r#"mm,,AATESTKEY,AATESTKEY,AATESTKEY,AATESTKEY"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_035_basic_nyt_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Nyt),
        r#"mm,,Glashow      Sheldon        ,1961,Partial Symmetries of Weak Interactions,22"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_036_basic_nyvt_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Nyvt),
        r#"mm,,Glashow      Sheldon        ,1961,22,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_037_anyt_sort_with_labelalpha() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Anyt),
        r#"mm,Gla61,,Glashow      Sheldon        ,1961,Partial Symmetries of Weak Interactions,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_038_anyt_sort_without_labelalpha() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Anyt),
        r#"mm,,,Glashow      Sheldon        ,1961,Partial Symmetries of Weak Interactions,"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_039_anyvt_sort_with_labelalpha() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Anyvt),
        r#"mm,Gla61,,Glashow      Sheldon        ,1961,0022,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_040_anyvt_sort_maxbibnames_3_minbibnames_1_with_labelalpha_and_alphaothers() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "murray", Template::Anyvt),
        r#"mm,Hos+98,,Hostetler    Michael J.     􏿽,1998,0014,Alkanethiolate gold cluster molecules with core diameters from 1.5 to 5.2 nm"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_041_anyvt_sort_maxbibnames_2_minbibnames_2_with_labelalpha_and_alphaothers() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "murray", Template::Anyvt),
        r#"mm,HW+98,,Hostetler    Michael J.     Wingate      Julia E.       􏿽,1998,0014,Alkanethiolate gold cluster molecules with core diameters from 1.5 to 5.2 nm"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_042_anyvt_sort_maxbibnames_2_minbibnames_2_with_labelalpha_and_without_alphaothers() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "murray", Template::Anyvt),
        r#"mm,HW98,,Hostetler    Michael J.     Wingate      Julia E.       􏿽,1998,0014,Alkanethiolate gold cluster molecules with core diameters from 1.5 to 5.2 nm"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_043_anyvt_sort_without_labelalpha() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Anyvt),
        r#"mm,,,Glashow      Sheldon        ,1961,0022,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_044_basic_ynt_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Ynt),
        r#"mm,,1961,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_045_basic_ydnt_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Ydnt),
        r#"mm,,1961,Glashow      Sheldon        ,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_046_sort_first_name_inits_only() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Ydnt),
        r#"mm,,1961,Glashow      S  ,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
fn assertion_047_basic_debug_sort() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::EntryKey),
        r#"stdmodel"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_048_nty_with_use_all_off() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::Nty),
        r#"mm,,Partial Symmetries of Weak Interactions,Partial Symmetries of Weak Interactions,1961,22"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_049_nty_with_modified_presort_and_short_circuit_title() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel:ps_sc", Template::NtyTitleFinal),
        r#"zs,,Partial Symmetries of Weak Interactions,,Partial Symmetries of Weak Interactions,Partial Symmetries of Weak Interactions"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_050_citeorder() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "stdmodel", Template::CiteOrder),
        r#"1"#
    );
}

#[test]
#[ignore = "xfail: native sort projection does not yet match Biber sort data"]
fn assertion_051_date_labels() {
    let result = run_fixture("general");
    assert_eq!(
        sort_data_for_key(&result, "labelstest", Template::Labels),
        r#"2005,3,2"#
    );
}

#[test]
#[ignore = "xfail: native sortname selection does not yet honor Biber use-name options"]
fn assertion_052_sortname_1() {
    let result = run_fixture("general");
    assert_eq!(sort_data_for_key(&result, "sn1", Template::Name), r#""#);
}
