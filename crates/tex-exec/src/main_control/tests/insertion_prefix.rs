//! TeX82 §1099 prefix scope and body entry; pdfTeX adds the `pre` keyword.
use super::*;

#[test]
fn insertion_brace_expansion_owns_the_group_before_body_mode_entry() {
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        for head in [r"\insert0", r"\vadjust"] {
            for (prefix, body) in [
                (r"\relax\csname scopedprefix\endcsname", ""),
                ("", r"\csname scopedprefix\endcsname"),
            ] {
                crate::test_harness::with_nonstop_plain_universe(|stores| {
                    let mut control = match profile {
                        CommandProfile::TEX82 => MainControl::tex82_initex(stores),
                        CommandProfile::ETEX26 => etex_initex(stores),
                        _ => pdftex_initex(stores),
                    };
                    // A containing hbox also makes vadjust legal without a font.
                    let source = format!(
                        "\\setbox0=\\hbox{{{head}{prefix}{{{body}}}\\ifx\\scopedprefix\\undefined\\global\\count0=1 \\else\\global\\count0=2 \\fi}}\\end"
                    );
                    register_source(&mut control, source.as_bytes());
                    run_to_end(&mut control, stores);
                    assert_eq!(stores.count(0).expect("scope result"), 1, "{source}");
                    assert!(
                        !terminal_text(stores).contains("! "),
                        "{}",
                        terminal_text(stores)
                    );
                });
            }
        }
    }
}

#[test]
fn insertion_brace_expansion_observes_insertion_group_and_enclosing_mode() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\let\brace={
            \def\open{\ifnum\currentgrouptype=11 \else\errmessage{wrong group}\fi
                \ifhmode\else\errmessage{wrong prefix mode}\fi\brace}
            \everyvbox={\errmessage{unexpected everyvbox}}
            \def\body{\ifvmode\else\errmessage{wrong body mode}\fi\global\advance\count0 by1 }
            \setbox0=\hbox{\insert0\relax\open\body}\vadjust pre\relax\open\body}}\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(stores.count(0).expect("both bodies"), 2);
        assert!(
            !terminal_text(stores).contains("! "),
            "{}",
            terminal_text(stores)
        );
    });
}

#[test]
fn insertion_class_expansion_stays_outer_and_reserved_class_reports_before_group() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\tracinggroups=1\tracingonline=1
            \setbox0=\hbox{\insert0\csname outerprefix\endcsname{}
                \ifx\outerprefix\relax\global\count0=1 \fi
                \insert255\relax\csname innerprefix\endcsname{}
                \ifx\innerprefix\undefined\global\count1=1 \fi}\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(stores.count(0).expect("class expansion is outer"), 1);
        assert_eq!(stores.count(1).expect("brace expansion is inner"), 1);
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("! ").count(), 1, "{terminal}");
        let report = terminal
            .find("You can't \\insert255")
            .expect("reserved class report");
        let enter = terminal
            .rfind("entering insert group")
            .expect("insertion group trace");
        assert!(report < enter, "{terminal}");
        let classes: Vec<_> = box_child_nodes(stores, 0)
            .into_iter()
            .filter_map(|node| {
                if let Node::Ins { class, .. } = node {
                    Some(class)
                } else {
                    None
                }
            })
            .collect();
        assert!(classes == [0, 0], "{classes:?}");
    });
}
