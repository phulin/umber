// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "extratitleyear";
    #[ignore = "xfail: extra-title-year metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_same_title_same_year {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"L1"#####, field: r#####"extratitleyear"#####, expected: Some(r#####"1"#####) },
    }
    #[ignore = "xfail: extra-title-year metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_same_title_same_year {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"L2"#####, field: r#####"extratitleyear"#####, expected: Some(r#####"2"#####) },
    }
    3 assertion_003_no_title_same_year {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"L3"#####, field: r#####"extratitle"#####, expected: None },
    }
    4 assertion_004_same_title_different_year {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"L4"#####, field: r#####"extratitleyear"#####, expected: None },
    }
    5 assertion_005_different_labeltitle_same_year {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"L5"#####, field: r#####"extratitleyear"#####, expected: None },
    }
    6 assertion_006_different_years_due_to_range_ends_1 {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"LY1"#####, field: r#####"extratitleyear"#####, expected: None },
    }
    7 assertion_007_different_years_due_to_range_ends_1 {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"LY2"#####, field: r#####"extratitleyear"#####, expected: None },
    }
    8 assertion_008_different_years_due_to_range_ends_1 {
        control: r#####"extratitleyear.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"LY3"#####, field: r#####"extratitleyear"#####, expected: None },
    }
}
