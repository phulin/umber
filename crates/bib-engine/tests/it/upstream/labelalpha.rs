// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "labelalpha";
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_useprefix_0_so_not_in_label {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"prefix1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Vaa99"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_default_prefix_settings_entry_prefix1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"prefix1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"vdVaa99"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    3 assertion_003_maxalphanames_1_minalphanames_1_entry_l1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe95"#####) },
    }
    4 assertion_004_maxalphanames_1_minalphanames_1_entry_l1_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"l1"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    5 assertion_005_maxalphanames_1_minalphanames_1_entry_l2_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    6 assertion_006_maxalphanames_1_minalphanames_1_entry_l2_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    7 assertion_007_maxalphanames_1_minalphanames_1_entry_l3_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    8 assertion_008_maxalphanames_1_minalphanames_1_entry_l3_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    9 assertion_009_maxalphanames_1_minalphanames_1_entry_l4_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    10 assertion_010_maxalphanames_1_minalphanames_1_entry_l4_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extraalpha"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    11 assertion_011_maxalphanames_1_minalphanames_1_entry_l5_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    12 assertion_012_maxalphanames_1_minalphanames_1_entry_l5_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extraalpha"#####, expected: Some(r#####"4"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    13 assertion_013_maxalphanames_1_minalphanames_1_entry_l6_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    14 assertion_014_maxalphanames_1_minalphanames_1_entry_l6_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extraalpha"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    15 assertion_015_maxalphanames_1_minalphanames_1_entry_l7_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    16 assertion_016_maxalphanames_1_minalphanames_1_entry_l7_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extraalpha"#####, expected: Some(r#####"6"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    17 assertion_017_maxalphanames_1_minalphanames_1_entry_l8_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sha85"#####) },
    }
    18 assertion_018_maxalphanames_1_minalphanames_1_entry_l8_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"extraalpha"#####, expected: None },
    }
    19 assertion_019_l9_extraalpha_unset_due_to_shorthand {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L9"#####, field: r#####"extraalpha"#####, expected: None },
    }
    20 assertion_020_l10_extraalpha_unset_due_to_shorthand {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L10"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    21 assertion_021_year_with_range_needs_label_differentiating_from_individual_volumes_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"knuth:ct"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    22 assertion_022_year_with_range_needs_label_differentiating_from_individual_volumes_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"knuth:ct:a"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    23 assertion_023_year_with_range_needs_label_differentiating_from_individual_volumes_3 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"knuth:ct:b"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    24 assertion_024_year_with_range_needs_label_differentiating_from_individual_volumes_4 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"knuth:ct:c"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    25 assertion_025_default_ignore {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ignore1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"OTo07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    26 assertion_026_default_no_ignore_spaces {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ignore2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"De 07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    27 assertion_027_maxalphanames_2_minalphanames_1_entry_l1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe95"#####) },
    }
    28 assertion_028_maxalphanames_2_minalphanames_1_entry_l1_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"l1"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    29 assertion_029_maxalphanames_2_minalphanames_1_entry_l2_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    30 assertion_030_maxalphanames_2_minalphanames_1_entry_l2_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    31 assertion_031_maxalphanames_2_minalphanames_1_entry_l3_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    32 assertion_032_maxalphanames_2_minalphanames_1_entry_l3_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    33 assertion_033_maxalphanames_2_minalphanames_1_entry_l4_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    34 assertion_034_maxalphanames_2_minalphanames_1_entry_l4_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    35 assertion_035_maxalphanames_2_minalphanames_1_entry_l5_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    36 assertion_036_maxalphanames_2_minalphanames_1_entry_l5_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    37 assertion_037_maxalphanames_2_minalphanames_1_entry_l6_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    38 assertion_038_maxalphanames_2_minalphanames_1_entry_l6_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extraalpha"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    39 assertion_039_maxalphanames_2_minalphanames_1_entry_l7_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    40 assertion_040_maxalphanames_2_minalphanames_1_entry_l7_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extraalpha"#####, expected: Some(r#####"4"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    41 assertion_041_maxalphanames_2_minalphanames_1_entry_l8_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sha85"#####) },
    }
    42 assertion_042_maxalphanames_2_minalphanames_1_entry_l8_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    43 assertion_043_maxalphanames_2_minalphanames_2_entry_l1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe95"#####) },
    }
    44 assertion_044_maxalphanames_2_minalphanames_2_entry_l1_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"l1"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    45 assertion_045_maxalphanames_2_minalphanames_2_entry_l2_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    46 assertion_046_maxalphanames_2_minalphanames_2_entry_l2_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    47 assertion_047_maxalphanames_2_minalphanames_2_entry_l3_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    48 assertion_048_maxalphanames_2_minalphanames_2_entry_l3_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    49 assertion_049_maxalphanames_2_minalphanames_2_entry_l4_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    50 assertion_050_maxalphanames_2_minalphanames_2_entry_l4_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    51 assertion_051_maxalphanames_2_minalphanames_2_entry_l5_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    52 assertion_052_maxalphanames_2_minalphanames_2_entry_l5_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    53 assertion_053_maxalphanames_2_minalphanames_2_entry_l6_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DS+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    54 assertion_054_maxalphanames_2_minalphanames_2_entry_l6_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    55 assertion_055_maxalphanames_2_minalphanames_2_entry_l7_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DS+95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    56 assertion_056_maxalphanames_2_minalphanames_2_entry_l7_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    57 assertion_057_maxalphanames_2_minalphanames_2_entry_l8_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sha85"#####) },
    }
    58 assertion_058_maxalphanames_2_minalphanames_2_entry_l8_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    59 assertion_059_maxalphanames_3_minalphanames_1_entry_l1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Doe95"#####) },
    }
    60 assertion_060_maxalphanames_3_minalphanames_1_entry_l1_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    61 assertion_061_maxalphanames_3_minalphanames_1_entry_l2_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    62 assertion_062_maxalphanames_3_minalphanames_1_entry_l2_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    63 assertion_063_maxalphanames_3_minalphanames_1_entry_l3_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DA95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    64 assertion_064_maxalphanames_3_minalphanames_1_entry_l3_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    65 assertion_065_maxalphanames_3_minalphanames_1_entry_l4_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DAE95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    66 assertion_066_maxalphanames_3_minalphanames_1_entry_l4_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    67 assertion_067_maxalphanames_3_minalphanames_1_entry_l5_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DAE95"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    68 assertion_068_maxalphanames_3_minalphanames_1_entry_l5_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    69 assertion_069_maxalphanames_3_minalphanames_1_entry_l6_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DSE95"#####) },
    }
    70 assertion_070_maxalphanames_3_minalphanames_1_entry_l6_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    71 assertion_071_maxalphanames_3_minalphanames_1_entry_l7_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"DSJ95"#####) },
    }
    72 assertion_072_maxalphanames_3_minalphanames_1_entry_l7_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    73 assertion_073_maxalphanames_3_minalphanames_1_entry_l8_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sha85"#####) },
    }
    74 assertion_074_maxalphanames_3_minalphanames_1_entry_l8_extraalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"extraalpha"#####, expected: None },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    75 assertion_075_testing_compound_lastnames_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LDN1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"VUR89"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    76 assertion_076_testing_compound_lastnames_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LDN2"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"VU45"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    77 assertion_077_testing_with_multiple_pre_and_main_and_width_side_override {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LDN3"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"VisvSJRu45"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    78 assertion_078_prefix_labelalpha_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L11"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"vRan22"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    79 assertion_079_prefix_labelalpha_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L12"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"vRvB2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    80 assertion_080_per_type_labelalpha_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L13"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"vRa+-ksUnV"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    81 assertion_081_per_type_labelalpha_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L14"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Alabel-ksUnW"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    82 assertion_082_labelalpha_disambiguation_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L15"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AccBrClim"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    83 assertion_083_labelalpha_disambiguation_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L16"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AccBaClim"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    84 assertion_084_labelalpha_disambiguation_2a {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L16a"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AccBaClim"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    85 assertion_085_labelalpha_disambiguation_2c {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L16"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    86 assertion_086_labelalpha_disambiguation_2d {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L16a"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    87 assertion_087_labelalpha_disambiguation_3 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L17"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AckBaClim"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    88 assertion_088_custom_labelalpha_extradate_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L17a"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    89 assertion_089_labelalpha_disambiguation_4 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L18"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AgChLa"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    90 assertion_090_labelalpha_disambiguation_5 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L19"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AgConLe"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    91 assertion_091_labelalpha_disambiguation_6 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L20"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AgCouLa"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    92 assertion_092_labelalpha_disambiguation_7 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L21"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"BoConEdb"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    93 assertion_093_labelalpha_disambiguation_8 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L22"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"BoConEm"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    94 assertion_094_labelalpha_disambiguation_9 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L23"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sa"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    95 assertion_095_labelalpha_disambiguation_10 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L18"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Agas/Cha/Laver"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    96 assertion_096_labelalpha_disambiguation_11 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L19"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Agas/Con/Lendl"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    97 assertion_097_labelalpha_disambiguation_12 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L20"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Agas/Cou/Laver"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    98 assertion_098_labelalpha_list_disambiguation_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L18"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"AChL"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    99 assertion_099_labelalpha_list_disambiguation_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L19"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"ACoL"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    100 assertion_100_labelalpha_list_disambiguation_3 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L20"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"ACL"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    101 assertion_101_labelalpha_list_disambiguation_4 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L21"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"BCEd"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    102 assertion_102_labelalpha_list_disambiguation_5 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L22"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"BCE"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    103 assertion_103_labelalpha_list_disambiguation_6 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L24"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Z"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    104 assertion_104_labelalpha_list_disambiguation_7 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L25"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"ZX"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    105 assertion_105_labelalpha_list_disambiguation_8 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L26"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"ZX"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    106 assertion_106_title_in_braces_with_utf_8_char_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"title1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Tït"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    107 assertion_107_extraalpha_ne_extradate_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schmidt2007"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sch+07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    108 assertion_108_extraalpha_ne_extradate_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schmidt2007"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    109 assertion_109_extraalpha_ne_extradate_3 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schmidt2007a"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sch07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    110 assertion_110_extraalpha_ne_extradate_4 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schmidt2007a"#####, field: r#####"extraalpha"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    111 assertion_111_extraalpha_ne_extradate_5 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schnee2007"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sch+07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    112 assertion_112_extraalpha_ne_extradate_6 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schnee2007"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    113 assertion_113_extraalpha_ne_extradate_7 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schnee2007a"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"Sch07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    114 assertion_114_extraalpha_ne_extradate_8 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schnee2007a"#####, field: r#####"extraalpha"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    115 assertion_115_entrykey_label_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"Schmidt2007"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"SCH"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    116 assertion_116_labeldate_test_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"labelstest"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"200532"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    117 assertion_117_pad_test_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"padtest"#####, field: r#####"labelalpha"#####, expected: Some(r#####"\&Al\_\_{\textasciitilde}{\textasciitilde}T07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    118 assertion_118_pad_test_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"padtest"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"&Al__~~T07"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    119 assertion_119_skip_width_test_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"skipwidthtest1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"OToolOToole"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    120 assertion_120_compound_and_string_length_entry_prefix1_labelalpha {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"prefix1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"vadeVaaThin"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    121 assertion_121_name_range_test_1 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"rangetest1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"WAXAYAZA.VEWEXE+.VTWT.XFYFZF.WH+"#####) },
    }
    #[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
    122 assertion_122_name_range_test_2 {
        control: r#####"labelalpha.bcf"#####,
        options: &[
                (r#####"maxalphanames"#####, r#####"10"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"10"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"rangetest1"#####, field: r#####"sortlabelalpha"#####, expected: Some(r#####"VWXYZ..V/W/X/Y/Z"#####) },
    }
}
