//! Shared (copy-on-write) box bodies: `docs/shared_box_closures.md`.
//!
//! Each case copies a register large enough to freeze, then checks that every
//! reader sees value semantics and that the frozen body retires once the last
//! box naming it is gone.

use tex_state::GenerationBrand;

use super::*;

/// A flat vbox body of distinct kerns and penalties, large enough to freeze.
fn big_body() -> String {
    (1..=48)
        .map(|index| format!("\\kern{index}sp\\penalty{index}"))
        .collect()
}

fn summary(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| match node {
            Node::Kern { amount, .. } => format!("k{}", amount.raw()),
            Node::Penalty(value) => format!("p{value}"),
            Node::Mark { .. } => "mark".into(),
            Node::HList(_) => "hbox".into(),
            Node::VList(_) => "vbox".into(),
            Node::Glue { .. } => "glue".into(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn run_shared(source: &str, test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>)) {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, source.as_bytes());
        run_to_end(&mut control, stores);
        test(stores);
    });
}

fn expected_body() -> Vec<String> {
    (1..=48)
        .flat_map(|index| [format!("k{index}"), format!("p{index}")])
        .collect()
}

#[test]
fn copies_share_one_frozen_body_and_read_identically() {
    let source = format!(
        "\\setbox0=\\vbox{{{}}}\\setbox1=\\copy0\\setbox2=\\vbox{{\\copy0}}\\end",
        big_body()
    );
    run_shared(&source, |stores| {
        let counters = stores.frozen_region_counters();
        assert_eq!(counters.frozen, 1, "{counters:?}");
        assert!(counters.shared_bodies >= 1, "{counters:?}");
        assert_eq!(summary(&box_child_nodes(stores, 0)), expected_body());
        assert_eq!(summary(&box_child_nodes(stores, 1)), expected_body());
        let wrapped = box_child_nodes(stores, 2);
        let [Node::VList(inner)] = wrapped.as_slice() else {
            panic!("box 2 wraps one copied vbox");
        };
        assert_eq!(summary(&page_vec(stores, inner.children)), expected_body());
    });
}

#[test]
fn frozen_body_retires_with_its_last_borrower() {
    let source = format!(
        "\\setbox0=\\vbox{{{}}}\\setbox1=\\copy0\\setbox2=\\copy1\
         \\setbox0=\\box9\\setbox1=\\box9\\setbox2=\\box9\\hbox{{}}\\penalty-10000\\end",
        big_body()
    );
    run_shared(&source, |stores| {
        let counters = stores.frozen_region_counters();
        assert_eq!(counters.frozen, 1, "{counters:?}");
        assert_eq!(counters.retired, 1, "{counters:?}");
    });
}

#[test]
fn edits_to_either_side_never_reach_the_other() {
    let source = format!(
        "\\setbox0=\\vbox{{{}}}\\setbox1=\\copy0\\wd0=5pt\
         \\setbox0=\\vbox{{\\unvbox0\\kern99sp}}\
         \\setbox1=\\vbox{{\\unvbox1\\unskip\\unpenalty\\unkern}}\\end",
        big_body()
    );
    run_shared(&source, |stores| {
        let mut original = expected_body();
        original.push("k99".into());
        assert_eq!(summary(&box_child_nodes(stores, 0)), original);
        let mut trimmed = expected_body();
        trimmed.truncate(trimmed.len() - 2);
        assert_eq!(summary(&box_child_nodes(stores, 1)), trimmed);
    });
}

#[test]
fn unvcopy_lastbox_and_vsplit_materialize_borrowed_material() {
    let source = format!(
        "\\setbox0=\\vbox{{{}\\hbox{{\\kern7sp}}}}\\setbox1=\\copy0\
         \\setbox3=\\vbox{{\\unvcopy0\\global\\setbox4=\\lastbox}}\
         \\splittopskip=0pt\\setbox5=\\vsplit1 to 0pt\\end",
        big_body()
    );
    run_shared(&source, |stores| {
        let mut with_box = expected_body();
        with_box.push("hbox".into());
        assert_eq!(summary(&box_child_nodes(stores, 0)), with_box);
        assert_eq!(summary(&box_child_nodes(stores, 3)), expected_body());
        assert_eq!(summary(&box_child_nodes(stores, 4)), ["k7"]);
        let split = summary(&box_child_nodes(stores, 5));
        let rest = summary(&box_child_nodes(stores, 1));
        assert!(
            !split.is_empty() && !rest.is_empty(),
            "{split:?} / {rest:?}"
        );
        assert_eq!(summary(&box_child_nodes(stores, 0)), with_box);
    });
}

#[test]
fn hbox_copy_in_vertical_mode_migrates_marks_from_a_borrowed_body() {
    let source = format!(
        "\\setbox0=\\hbox{{\\mark{{a}}{}}}\\setbox1=\\vbox{{\\copy0}}\\end",
        big_body()
    );
    run_shared(&source, |stores| {
        assert_eq!(stores.frozen_region_counters().frozen, 1);
        let outer = summary(&box_child_nodes(stores, 1));
        // TeX82 §1076 appends migrated material after the box.
        assert_eq!(outer, ["hbox", "mark"]);
        let mut body = vec!["mark".to_owned()];
        body.extend(expected_body());
        assert_eq!(summary(&box_child_nodes(stores, 0)), body);
    });
}
