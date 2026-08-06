// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "extradate";
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_entry_l1_one_name_first_in_1995 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_entry_l2_one_name_second_in_1995 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    3 assertion_003_entry_l3_one_name_third_in_1995 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extradate"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    4 assertion_004_entry_l4_two_names_first_in_1995 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    5 assertion_005_entry_l5_two_names_second_in_1995 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    6 assertion_006_entry_l6_two_names_first_in_1996 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    7 assertion_007_entry_l7_two_names_second_in_1996 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    8 assertion_008_same_name_no_year_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"nodate1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    9 assertion_009_same_name_no_year_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"nodate2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    10 assertion_010_entry_l8_one_name_only_in_year {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"extradate"#####, expected: None },
    }
    11 assertion_011_entry_l9_no_name_same_year_as_another_with_no_name {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L9"#####, field: r#####"extradate"#####, expected: None },
    }
    12 assertion_012_entry_l10_no_name_same_year_as_another_with_no_name {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L10"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    13 assertion_013_entry_companion1_names_truncated_to_same_as_another_entry_in_same_year {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"companion1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    14 assertion_014_entry_companion2_names_truncated_to_same_as_another_entry_in_same_year {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"companion2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    15 assertion_015_entry_companion3_one_name_same_year_as_truncated_names {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"companion3"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    16 assertion_016_entry_vangennep_useprefix_does_makes_it_different {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"vangennep"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    17 assertion_017_entry_gennep_different_from_prefix_name {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"gennep"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    18 assertion_018_date_range_means_no_extradate_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LY1"#####, field: r#####"extradate"#####, expected: None },
    }
    19 assertion_019_date_range_means_no_extradate_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LY2"#####, field: r#####"extradate"#####, expected: None },
    }
    20 assertion_020_date_range_means_no_extradate_3 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"LY3"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    21 assertion_021_labeldatesource_string_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"nodate1"#####, field: r#####"labeldatesource"#####, expected: Some(r#####"nodate"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    22 assertion_022_labeldatesource_string_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"nodate2"#####, field: r#####"labeldatesource"#####, expected: Some(r#####"nodate"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    23 assertion_023_labelyear_scope_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    24 assertion_024_labelyear_scope_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    25 assertion_025_labelyear_scope_1a {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed1"#####, field: r#####"extradatescope"#####, expected: Some(r#####"labelyear"#####) },
    }
    26 assertion_026_labelyear_scope_3 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed7"#####, field: r#####"extradate"#####, expected: None },
    }
    27 assertion_027_labelyear_scope_4 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed8"#####, field: r#####"extradate"#####, expected: None },
    }
    28 assertion_028_labelmonth_scope_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed1"#####, field: r#####"extradate"#####, expected: None },
    }
    29 assertion_029_labelmonth_scope_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed2"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    30 assertion_030_labelmonth_scope_1a {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed1"#####, field: r#####"extradatescope"#####, expected: Some(r#####"labelmonth"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    31 assertion_031_labelmonth_scope_3 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed3"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    32 assertion_032_labelmonth_scope_4 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed4"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    33 assertion_033_labelminute_scope_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed5"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    34 assertion_034_labelminute_scope_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed6"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    35 assertion_035_labelminute_scope_1a {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed5"#####, field: r#####"extradatescope"#####, expected: Some(r#####"labelminute"#####) },
    }
    36 assertion_036_labelminute_scope_3 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed1"#####, field: r#####"extradate"#####, expected: None },
    }
    37 assertion_037_labelminute_scope_4 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed2"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    38 assertion_038_year_scope_1 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"extradatespec"#####, r#####"year"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed7"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
    39 assertion_039_year_scope_2 {
        control: r#####"extradate.bcf"#####,
        options: &[
                (r#####"extradatespec"#####, r#####"year"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ed8"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
}
