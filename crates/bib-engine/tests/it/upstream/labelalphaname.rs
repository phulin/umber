// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use super::compatibility::{OutputExpectation, compatibility_cases};

compatibility_cases! {
    module "labelalphaname";
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    1 assertion_001_labelalphaname_global_template {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant1"#####, field: r#####"labelalpha"#####, expected: Some(r#####"Smi"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    2 assertion_002_labelalphaname_dlist_template {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant1"#####, field: r#####"labelalpha"#####, expected: Some(r#####"AS"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    3 assertion_003_labelalphaname_entry_template {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant2"#####, field: r#####"labelalpha"#####, expected: Some(r#####"ArSm"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    4 assertion_004_labelalphaname_namelist_template {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant3"#####, field: r#####"labelalpha"#####, expected: Some(r#####"ArtSmi"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    5 assertion_005_labelalphaname_name_template {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant4"#####, field: r#####"labelalpha"#####, expected: Some(r#####"ArthSmit"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    6 assertion_006_labelalphaname_name_template_compound {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant5"#####, field: r#####"labelalpha"#####, expected: Some(r#####"GRW"#####) },
    }
    #[ignore = "xfail: label-alpha-name metadata differs from the Biber 2.22 expectation"]
    7 assertion_007_labelalphaname_name_template_hyphen {
        control: r#####"labelalphaname.bcf"#####,
        options: &[],
        output: OutputExpectation::Field { entry: r#####"lant6"#####, field: r#####"labelalpha"#####, expected: Some(r#####"GRW"#####) },
    }
}
