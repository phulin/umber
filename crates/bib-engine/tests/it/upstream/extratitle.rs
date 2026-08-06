// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "extratitle";
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_same_name_same_title_1 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"extratitle"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_same_name_same_title_2 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extratitle"#####, expected: Some(r#####"2"#####) },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    3 assertion_003_no_name_same_title_1 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extratitle"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    4 assertion_004_no_name_same_title_2 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extratitle"#####, expected: Some(r#####"2"#####) },
    }
    5 assertion_005_no_name_same_title_as_with_name_1 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extratitle"#####, expected: None },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    6 assertion_006_no_name_same_shorttitle_title_1 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L6"#####, field: r#####"extratitle"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    7 assertion_007_no_name_same_shorttitle_title_2 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L7"#####, field: r#####"extratitle"#####, expected: Some(r#####"2"#####) },
    }
    8 assertion_008_singletitle_test_1 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L8"#####, field: r#####"singletitle"#####, expected: None },
    }
    9 assertion_009_singletitle_test_2 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L9"#####, field: r#####"singletitle"#####, expected: None },
    }
    #[ignore = "xfail: extra-title metadata differs from the Biber 2.22 expectation"]
    10 assertion_010_singletitle_test_3 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L10"#####, field: r#####"singletitle"#####, expected: Some(r#####"1"#####) },
    }
    11 assertion_011_singletitle_test_4 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L11"#####, field: r#####"singletitle"#####, expected: None },
    }
    12 assertion_012_singletitle_test_5 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L12"#####, field: r#####"singletitle"#####, expected: None },
    }
    13 assertion_013_singletitle_test_6 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"singletitle"#####, expected: None },
    }
    14 assertion_014_singletitle_test_7 {
        control: r#####"extratitle.bcf"#####,
        options: &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####)
            ],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"singletitle"#####, expected: None },
    }
}
