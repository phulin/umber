use std::collections::BTreeSet;
use std::fs;

use test_support::closed_case::FixtureCase;

const AREAS: &[(&str, &[&str])] = &[
    (
        "align",
        &[
            "basic_grid_tabskip",
            "display_halign",
            "empty_text_accent",
            "nested_halign",
            "noalign_rules",
            "omit_cells",
            "repeat_preamble",
            "span_template_order",
            "valign_leading_spaces",
            "valign_transpose",
        ],
    ),
    ("etex_exec", &["expansion_virtual_input"]),
    ("exec", &["math_component_recovery"]),
    (
        "expand",
        &[
            "csname_torture_loop",
            "delimited_args",
            "expandafter_noexpand",
            "ifx_hash_consing",
            "input_main",
            "nested_macros",
            "rendered_values",
            "skipped_conditionals_catcode",
        ],
    ),
    (
        "math",
        &[
            "accents",
            "alignment_leading_tabskip",
            "big_operators",
            "delimiter_commands",
            "display_eqnos",
            "exercise_formulas",
            "fixed_infinite_glue",
            "fractions",
            "inline_box",
            "left_right_growth",
            "mathchoice",
            "mathopen_boxed_delimiter",
            "radicals_growth",
            "relax_ligature_boundary",
            "rule_character_order",
            "scripts_styles",
            "spacing_pairs",
            "standalone_delimiters",
            "text_accent_slant",
        ],
    ),
    (
        "tex_exec",
        &[
            "after",
            "box_brace_aliases",
            "box_dimensions",
            "box_movement",
            "box_uncopy_badness",
            "every_box_hooks",
            "grouping",
            "hskip_penalty_recovery",
            "illegal_mag",
            "incompatible_mag",
            "insert_brace_aliases",
            "internal_dimension_params",
            "last_box",
            "lccode_selector_recovery",
            "pdf_compatibility_controls",
            "pdf_font_codes",
            "pdf_font_config",
            "pdf_form_diagnostics",
            "pdf_form_state",
            "pdf_form_traversal_diagnostics",
            "pdf_ignored_dimen_effects",
            "pdf_image_config",
            "pdf_metadata_config",
            "pdf_microtype_effects",
            "pdf_move_chars_warning",
            "pdf_output_policy",
            "prefixed_macro",
            "too_many",
            "wrong_close",
        ],
    ),
    (
        "typeset",
        &[
            "alignment_math_group_balance",
            "alignment_showlists_unset",
            "alignment_widths_spans",
            "display_math_machinery",
            "inline_math_boundaries",
            "inline_math_hbox_operator_penalties",
            "math_glue_provenance",
            "paragraph_line_shape",
            "paragraph_mark_migration",
            "paragraph_vadjust_migration",
            "paragraph_wide",
            "restricted_hbox_valign_leading_spaces",
            "vsplit_split_marks",
        ],
    ),
];

const ACTIVE_TEX82_EXEC: &[&str] = &[
    "after",
    "box_brace_aliases",
    "box_dimensions",
    "box_movement",
    "box_uncopy_badness",
    "every_box_hooks",
    "grouping",
    "hskip_penalty_recovery",
    "illegal_mag",
    "incompatible_mag",
    "insert_brace_aliases",
    "internal_dimension_params",
    "last_box",
    "lccode_selector_recovery",
    "prefixed_macro",
    "too_many",
    "wrong_close",
];

const ACTIVE_PDFTEX_EXEC: &[&str] = &[
    "pdf_compatibility_controls",
    "pdf_font_codes",
    "pdf_font_config",
    "pdf_form_diagnostics",
    "pdf_form_state",
    "pdf_form_traversal_diagnostics",
    "pdf_ignored_dimen_effects",
    "pdf_image_config",
    "pdf_metadata_config",
    "pdf_microtype_effects",
    "pdf_move_chars_warning",
    "pdf_output_policy",
];

#[test]
fn execution_minifixtures_are_closed_tracked_directories() {
    let repository = test_support::repository_root();
    let corpus = repository.join("tests/corpus");
    let mut identities = BTreeSet::new();

    for &(area, expected_cases) in AREAS {
        let area_path = corpus.join(area);
        let mut actual_cases = BTreeSet::new();
        for entry in fs::read_dir(&area_path).expect("read fixture area") {
            let entry = entry.expect("read fixture-area entry");
            let file_type = entry.file_type().expect("read fixture-area entry type");
            assert!(
                file_type.is_dir() && !file_type.is_symlink(),
                "{} must contain only regular case directories",
                area_path.display()
            );
            let case = entry.file_name().into_string().expect("case name is UTF-8");
            assert!(
                identities.insert(format!("{area}/{case}")),
                "duplicate case {area}/{case}"
            );
            actual_cases.insert(case.clone());
            let closed = FixtureCase::discover_tracked_at(
                &repository,
                format!("tests/corpus/{area}/{case}"),
                format!("{case}.tex"),
                (*area).to_owned(),
            )
            .unwrap_or_else(|error| panic!("{area}/{case} is not closed: {error:#}"));
            closed
                .read(&format!("{case}.tex"))
                .unwrap_or_else(|error| panic!("{area}/{case} lacks its source: {error:#}"));
        }
        assert_eq!(
            actual_cases,
            expected_cases
                .iter()
                .map(|case| (*case).to_owned())
                .collect(),
            "{area} disposition census changed"
        );
    }
    assert_eq!(identities.len(), 81, "execution case census changed");
}

#[test]
fn retained_executor_cases_have_exact_active_ownership() {
    let tex_exec = expected_cases("tex_exec");
    let classified_tex_exec = ACTIVE_TEX82_EXEC
        .iter()
        .chain(ACTIVE_PDFTEX_EXEC)
        .copied()
        .collect();
    assert_eq!(
        tex_exec, classified_tex_exec,
        "tex_exec cases must have active behavioral ownership"
    );
}

fn expected_cases(area: &str) -> BTreeSet<&'static str> {
    AREAS
        .iter()
        .find_map(|(candidate, cases)| (*candidate == area).then_some(*cases))
        .expect("classified execution area exists")
        .iter()
        .copied()
        .collect()
}
