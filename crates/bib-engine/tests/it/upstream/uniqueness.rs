// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "uniqueness";
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_uniquename_requiring_full_name_expansion_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un1"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_uniquename_requiring_full_name_expansion_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un2"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    3 assertion_003_uniquename_requiring_full_name_expansion_3 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    4 assertion_004_uniquename_requiring_initials_name_expansion_per_namelist_uniquename_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un3"#####, name_index: 1, assignment: r#####"un"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    5 assertion_005_uniquename_requiring_initials_name_expansion_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    6 assertion_006_per_entry_uniquename {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un4a"#####, name_index: 1, assignment: r#####"un"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    7 assertion_007_namehash_and_fullhash_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un6"#####, field: r#####"namehash"#####, expected: Some(r#####"f8169a157f8d9209961157b8d23902db"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    8 assertion_008_namehash_and_fullhash_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un6"#####, field: r#####"fullhash"#####, expected: Some(r#####"f8169a157f8d9209961157b8d23902db"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    9 assertion_009_fullnamehash_ignores_short_names_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un7"#####, field: r#####"namehash"#####, expected: Some(r#####"b33fbd3f3349d1536dbcc14664f2cbbd"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    10 assertion_010_fullnamehash_ignores_short_names_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un7"#####, field: r#####"fullhash"#####, expected: Some(r#####"f8169a157f8d9209961157b8d23902db"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    11 assertion_011_namehash_and_fullhash_3 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"test1"#####, field: r#####"namehash"#####, expected: Some(r#####"07df5c892ba1452776abee0a867591f2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    12 assertion_012_namehash_and_fullhash_4 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"test1"#####, field: r#####"fullhash"#####, expected: Some(r#####"637292dd2997a74c91847f1ec5081a46"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    13 assertion_013_uniquename_with_full_and_repeat_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"untf1"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    14 assertion_014_uniquename_with_full_and_repeat_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"untf2"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    15 assertion_015_uniquename_with_full_and_repeat_3 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"untf3"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    16 assertion_016_prefix_suffix_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp1"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    17 assertion_017_prefix_suffix_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp2"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    18 assertion_018_prefix_suffix_3 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp3"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    19 assertion_019_prefix_suffix_4 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    20 assertion_020_prefix_suffix_5 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    21 assertion_021_prefix_suffix_6 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp6"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    22 assertion_022_prefix_suffix_7 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp7"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    23 assertion_023_prefix_suffix_8 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp8"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    24 assertion_024_prefix_suffix_9 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"sp9"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    25 assertion_025_uniquename_with_inits_and_repeat_1 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"unt1"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    26 assertion_026_uniquename_with_inits_and_repeat_2 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"unt2"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    27 assertion_027_uniquename_with_inits_and_repeat_3 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"unt3"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    28 assertion_028_uniquename_with_inits_and_repeat_4 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"unt4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    29 assertion_029_uniquename_with_inits_and_repeat_5 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"unt5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    30 assertion_030_namehash_and_fullhash_5 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall3"#####, field: r#####"namehash"#####, expected: Some(r#####"f1c5973adbc2e674fa4d98164c9ba5d5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    31 assertion_031_namehash_and_fullhash_6 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall3"#####, field: r#####"fullhash"#####, expected: Some(r#####"f1c5973adbc2e674fa4d98164c9ba5d5"#####) },
    }
    32 assertion_032_uniquelist_edgecase_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall3"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    33 assertion_033_uniquelist_edgecase_2 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall4"#####, field: r#####"uniquelist"#####, expected: Some(r#####"6"#####) },
    }
    34 assertion_034_uniquename_0_due_to_mincitenames_truncation {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test2"#####, name_index: 1, assignment: r#####"un"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    35 assertion_035_uniquename_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    36 assertion_036_uniquename_2 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    37 assertion_037_uniquename_3 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    38 assertion_038_uniquename_4 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    39 assertion_039_uniquename_5 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    40 assertion_040_uniquename_6 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    41 assertion_041_uniquename_7 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 4, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    42 assertion_042_uniquename_8 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un10"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    43 assertion_043_uniquelist_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un8"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    44 assertion_044_uniquelist_2 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un9"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    45 assertion_045_uniquelist_3 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"un10"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    46 assertion_046_uniquelist_4 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unapa1"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    47 assertion_047_uniquelist_5 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unapa2"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    48 assertion_048_uniquelist_6 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"others1"#####, field: r#####"uniquelist"#####, expected: None },
    }
    49 assertion_049_uniquelist_7 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall1"#####, field: r#####"uniquelist"#####, expected: None },
    }
    50 assertion_050_uniquelist_8 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall2"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    51 assertion_051_uniquelist_9 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall5"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    52 assertion_052_uniquelist_10 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall6"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    53 assertion_053_uniquelist_11 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall7"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    54 assertion_054_uniquelist_12 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall8"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    55 assertion_055_uniquelist_13 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall9"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    56 assertion_056_per_namelist_uniquelist_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall9a"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    57 assertion_057_uniquelist_14 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall10"#####, field: r#####"uniquelist"#####, expected: Some(r#####"6"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    58 assertion_058_uniquelist_15 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall3"#####, field: r#####"uniquelist"#####, expected: Some(r#####"5"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    59 assertion_059_uniquelist_16 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"unall4"#####, field: r#####"uniquelist"#####, expected: Some(r#####"6"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    60 assertion_060_uniquelist_17 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ul01"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    61 assertion_061_uniquelist_18 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ul02"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    62 assertion_062_uniquelist_19 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"test3"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    63 assertion_063_uniquename_9 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test3"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    64 assertion_064_uniquename_10 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test3"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    65 assertion_065_uniquelist_20 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"test4"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    66 assertion_066_uniquename_11 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    67 assertion_067_uniquename_12 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test4"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    68 assertion_068_uniquelist_21 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"test5"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    69 assertion_069_uniquename_13 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    70 assertion_070_uniquename_14 {
        control: r#####"uniqueness1.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"test5"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    71 assertion_071_uniquename_sparse_1 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us1"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    72 assertion_072_uniquename_sparse_2 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us1"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    73 assertion_073_uniquename_sparse_3 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us2"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    74 assertion_074_uniquename_sparse_4 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us2"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    75 assertion_075_uniquename_sparse_5 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us3"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    76 assertion_076_uniquename_sparse_6 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us3"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    77 assertion_077_uniquename_sparse_7 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    78 assertion_078_uniquename_sparse_8 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us4"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    79 assertion_079_uniquename_sparse_9 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    80 assertion_080_uniquename_sparse_10 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us6"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    81 assertion_081_uniquename_sparse_11 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us6"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    82 assertion_082_uniquename_sparse_12 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us6"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    83 assertion_083_uniquename_sparse_13 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us7"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    84 assertion_084_uniquename_sparse_14 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us7"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    85 assertion_085_uniquename_sparse_15 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us8"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    86 assertion_086_uniquename_sparse_16 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us8"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    87 assertion_087_uniquename_sparse_17 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us9"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    88 assertion_088_uniquename_sparse_18 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us9"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    89 assertion_089_uniquename_sparse_19 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us10"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    90 assertion_090_uniquename_sparse_20 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us10"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    91 assertion_091_uniquename_sparse_21 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us11"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    92 assertion_092_uniquename_sparse_22 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us11"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    93 assertion_093_uniquename_sparse_23 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us12"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    94 assertion_094_uniquename_sparse_24 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us12"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    95 assertion_095_uniquename_sparse_25 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us13"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    96 assertion_096_uniquename_sparse_26 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us13"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    97 assertion_097_uniquename_sparse_27 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    98 assertion_098_uniquename_sparse_28 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    99 assertion_099_uniquename_sparse_29 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    100 assertion_100_uniquename_sparse_30 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    101 assertion_101_uniquename_sparse_31 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    102 assertion_102_uniquename_sparse_32 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    103 assertion_103_uniquename_sparse_33 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us16"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    104 assertion_104_uniquename_sparse_34 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us16"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    105 assertion_105_uniquename_sparse_35 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us16"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    106 assertion_106_uniquename_sparse_36 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"us16"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    107 assertion_107_uniquename_sparse_37 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us17"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    108 assertion_108_uniquename_sparse_38 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us17"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    109 assertion_109_uniquename_sparse_39 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us17"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    110 assertion_110_uniquename_sparse_40 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us17"#####, name_index: 4, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    111 assertion_111_uniquename_sparse_41 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"us17"#####, field: r#####"uniquelist"#####, expected: Some(r#####"4"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    112 assertion_112_uniquename_sparse_42 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us18"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    113 assertion_113_uniquename_sparse_43 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us19"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    114 assertion_114_uniquename_sparse_44 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"us18"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    115 assertion_115_uniquename_sparse_45 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"us19"#####, field: r#####"uniquelist"#####, expected: Some(r#####"4"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    116 assertion_116_uniquename_sparse_46 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    117 assertion_117_uniquename_sparse_47 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    118 assertion_118_uniquename_sparse_48 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    119 assertion_119_uniquename_sparse_49 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    120 assertion_120_uniquename_sparse_50 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    121 assertion_121_uniquename_sparse_51 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    122 assertion_122_uniquename_sparse_52 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us20"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    123 assertion_123_uniquename_sparse_53 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us21"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    124 assertion_124_uniquename_sparse_54 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us22"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    125 assertion_125_uniquename_sparse_55 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us23"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    126 assertion_126_uniquename_sparse_56 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us24"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    127 assertion_127_uniquename_sparse_57 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us25"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    128 assertion_128_uniquename_sparse_58 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    129 assertion_129_uniquename_sparse_59 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    130 assertion_130_uniquename_sparse_60 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us14"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    131 assertion_131_uniquename_sparse_61 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    132 assertion_132_uniquename_sparse_62 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    133 assertion_133_uniquename_sparse_63 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us15"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    134 assertion_134_uniquename_sparse_64 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us26"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    135 assertion_135_uniquename_sparse_65 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us27"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    136 assertion_136_uniquename_sparse_66 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us28"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    137 assertion_137_uniquename_sparse_67 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us29"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    138 assertion_138_uniquename_sparse_68 {
        control: r#####"uniqueness4.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"us30"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    139 assertion_139_uniquelist_strict_1 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls1"#####, field: r#####"uniquelist"#####, expected: None },
    }
    140 assertion_140_uniquelist_strict_2 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls2"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    141 assertion_141_uniquelist_strict_3 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls3"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    142 assertion_142_uniquelist_strict_4 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls4"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    143 assertion_143_uniquelist_strict_5 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls5"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    144 assertion_144_uniquelist_strict_6 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls6"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    145 assertion_145_uniquelist_strict_7 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls7"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    146 assertion_146_uniquelist_minyear_1 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ulmy1"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    147 assertion_147_uniquelist_minyear_2 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ulmy2"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    148 assertion_148_uniquelist_minyear_3 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ulmy3"#####, field: r#####"uniquelist"#####, expected: None },
    }
    149 assertion_149_uniquelist_strict_8 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls8"#####, field: r#####"uniquelist"#####, expected: None },
    }
    150 assertion_150_uniquelist_strict_9 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls9"#####, field: r#####"uniquelist"#####, expected: None },
    }
    151 assertion_151_uniquelist_strict_10 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls1"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    152 assertion_152_uniquelist_strict_11 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls10"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    153 assertion_153_uniquelist_strict_12 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls11"#####, field: r#####"uniquelist"#####, expected: Some(r#####"3"#####) },
    }
    154 assertion_154_uniquelist_strict_13 {
        control: r#####"uniqueness5.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"uls12"#####, field: r#####"uniquelist"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    155 assertion_155_extrayear_1 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    156 assertion_156_extrayear_2 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    157 assertion_157_extrayear_3 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    158 assertion_158_extrayear_4 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    159 assertion_159_extrayear_5 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    160 assertion_160_extrayear_6 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    161 assertion_161_extrayear_7 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"extradate"#####, expected: None },
    }
    162 assertion_162_extrayear_8 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"extradate"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    163 assertion_163_extrayear_9 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    164 assertion_164_extrayear_10 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    165 assertion_165_extrayear_11 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"extradate"#####, expected: None },
    }
    166 assertion_166_extrayear_12 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"extradate"#####, expected: None },
    }
    167 assertion_167_singletitle_1 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"singletitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    168 assertion_168_singletitle_2 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"singletitle"#####, expected: Some(r#####"1"#####) },
    }
    169 assertion_169_singletitle_3 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"singletitle"#####, expected: None },
    }
    170 assertion_170_singletitle_4 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"singletitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    171 assertion_171_singletitle_5 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"singletitle"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    172 assertion_172_singletitle_6 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"singletitle"#####, expected: Some(r#####"1"#####) },
    }
    173 assertion_173_uniquetitle_1 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"uniquetitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    174 assertion_174_uniquetitle_2 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"uniquetitle"#####, expected: Some(r#####"1"#####) },
    }
    175 assertion_175_uniquetitle_3 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"uniquetitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    176 assertion_176_uniquetitle_4 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"uniquetitle"#####, expected: Some(r#####"1"#####) },
    }
    177 assertion_177_uniquetitle_5 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"uniquetitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    178 assertion_178_uniquetitle_6 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"uniquetitle"#####, expected: Some(r#####"1"#####) },
    }
    179 assertion_179_uniquebaretitle_1 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey7"#####, field: r#####"uniquebaretitle"#####, expected: None },
    }
    180 assertion_180_uniquebaretitle_2 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey8"#####, field: r#####"uniquebaretitle"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    181 assertion_181_uniquebaretitle_3 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey9"#####, field: r#####"uniquebaretitle"#####, expected: Some(r#####"1"#####) },
    }
    182 assertion_182_uniquework_1 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"uniquework"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    183 assertion_183_uniquework_2 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"uniquework"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    184 assertion_184_uniquework_3 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"uniquework"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    185 assertion_185_uniquework_4 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"uniquework"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    186 assertion_186_uniquework_5 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"uniquework"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    187 assertion_187_uniquework_6 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"uniquework"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    188 assertion_188_extrayear_13 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey1"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    189 assertion_189_extrayear_14 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey2"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    190 assertion_190_extrayear_15 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey3"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    191 assertion_191_extrayear_16 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey4"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    192 assertion_192_extrayear_17 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey5"#####, field: r#####"extradate"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    193 assertion_193_extrayear_18 {
        control: r#####"uniqueness3.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"ey6"#####, field: r#####"extradate"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    194 assertion_194_forced_init_expansion_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    195 assertion_195_forced_init_expansion_2 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    196 assertion_196_forced_init_expansion_3 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    197 assertion_197_forced_init_expansion_4 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    198 assertion_198_forced_init_expansion_5 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    199 assertion_199_forced_init_expansion_6 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    200 assertion_200_forced_init_expansion_7 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 4, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    201 assertion_201_forced_init_expansion_8 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un10"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    202 assertion_202_forced_name_expansion_1 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    203 assertion_203_forced_name_expansion_2 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    204 assertion_204_forced_name_expansion_3 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un8"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    205 assertion_205_forced_name_expansion_4 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    206 assertion_206_forced_name_expansion_5 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 2, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    207 assertion_207_forced_name_expansion_6 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 3, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    208 assertion_208_forced_name_expansion_7 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un9"#####, name_index: 4, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    209 assertion_209_forced_name_expansion_8 {
        control: r#####"uniqueness2.bcf"#####,
        options: &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un10"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    210 assertion_210_uniquelist_duplicates_1 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"entry1a"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    211 assertion_211_uniquelist_duplicates_2 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"entry1b"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    212 assertion_212_uniquelist_duplicates_3 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"entry2a"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    213 assertion_213_uniquelist_duplicates_4 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"entry2b"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    214 assertion_214_uniquelist_duplicates_5 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"A"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    215 assertion_215_uniquelist_duplicates_6 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"B"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    216 assertion_216_uniquelist_duplicates_7 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[(r#####"uniquelist"#####, r#####"true"#####)],
        output: OutputExpectation::Field { entry: r#####"C"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    217 assertion_217_uniquelist_true_uniquename_false_1 {
        control: r#####"uniqueness6.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"false"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"C"#####, field: r#####"uniquelist"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    218 assertion_218_pluralothers_test_1 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"po1"#####, field: r#####"visiblecite"#####, expected: Some(r#####"4"#####) },
    }
    219 assertion_219_pluralothers_test_2 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"po1"#####, field: r#####"extraname"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    220 assertion_220_pluralothers_test_3 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"po3"#####, field: r#####"visiblecite"#####, expected: Some(r#####"4"#####) },
    }
    221 assertion_221_pluralothers_test_4 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"po3"#####, field: r#####"extraname"#####, expected: None },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    222 assertion_222_pluralothers_test_5 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::BblEntry { entry: r#####"po3"#####, expected: r#####"    \entry{po3}{book}{}{}
      \name{author}{4}{ul=4}{%
        {{un=1,uniquepart=given,hash=c2ab7e2b5663336cc4e65c8bcf1a280d}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={A.},
           giveni={A\bibinitperiod},
           givenun=1}}%
        {{un=0,uniquepart=base,hash=1f4cf713d86f6083087eb3085db7815a}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={B.},
           giveni={B\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=a44def9031aa70c9f458f5b47a34c451}{%
           family={Cuthbert},
           familyi={C\bibinitperiod},
           given={C.},
           giveni={C\bibinitperiod},
           givenun=0}}%
        {{un=1,uniquepart=given,hash=91876a448dc35952ca94dc92cee07f89}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={D.},
           giveni={D\bibinitperiod},
           givenun=1}}%
      }
      \strng{namehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{bibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorbibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authornamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \field{labelalpha}{Abr\textbf{+}22}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title One}
      \field{year}{2022}
      \field{dateera}{ce}
    \endentry
"##### },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    223 assertion_223_uniquename_minyearinit_1 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un1"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    224 assertion_224_uniquename_minyearinit_2 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un2"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    225 assertion_225_uniquename_minyearinit_3 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un3"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"0"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    226 assertion_226_uniquename_minyearinit_4 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un4"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
    227 assertion_227_uniquename_minyearinit_5 {
        control: r#####"uniqueness7.bcf"#####,
        options: &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::NameAssignment { entry: r#####"un5"#####, name_index: 1, assignment: r#####"un"#####, expected: Some(r#####"1"#####) },
    }
}
