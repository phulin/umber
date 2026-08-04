use super::*;

fn classify(channel: StreamChannel, oracle: &str, umber: &str) -> Option<DivergenceClass> {
    classify_divergence(channel, oracle.as_bytes(), umber.as_bytes())
}

fn mismatch(expected: &str, actual: &str) -> ChannelMismatch {
    ChannelMismatch {
        line: 1,
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    }
}

/// [`reclassify_no_error_channel`] is the audited fix and operates on only
/// the one positional mismatch pair, deliberately blind to what else is in
/// the transcript -- see this case's real committed evidence, and contrast
/// with `alignments/display-prevdepth` below, whose positional mismatch has
/// the *identical* shape but is correctly `umber2-alfh.16` (a missing
/// `Underfull \hbox` diagnostic, buried deeper in that transcript, that this
/// function never looks at and therefore never disturbs).
#[test]
fn reclassify_finds_file_paren_closed_early_from_the_pinned_mismatch_alone() {
    assert_eq!(
        reclassify_no_error_channel(&mismatch(
            "(./predicate-dispatch.tex",
            "(./predicate-dispatch.tex)"
        )),
        Some(DivergenceClass::FileParenClosedEarly)
    );
    assert_eq!(
        reclassify_no_error_channel(&mismatch(
            "(./misplaced-delimiter-recovery.tex",
            "(./misplaced-delimiter-recovery.tex )"
        )),
        Some(DivergenceClass::FileParenClosedEarly)
    );
    assert_eq!(
        reclassify_no_error_channel(&mismatch(
            "(./mode-dispatch.tex",
            "(./mode-dispatch.tex [1.1.1.1.1.1] )"
        )),
        Some(DivergenceClass::FileParenClosedEarly)
    );
}

#[test]
fn reclassify_finds_different_error_from_the_pinned_mismatch_alone() {
    assert_eq!(
        reclassify_no_error_channel(&mismatch("Runaway definition?", "! Too many }'s.")),
        Some(DivergenceClass::UmberRaisesUnexpectedError)
    );
}

#[test]
fn reclassify_confirms_genuine_no_error_from_the_pinned_mismatch_alone() {
    assert_eq!(
        reclassify_no_error_channel(&mismatch(
            "! This can't happen (256 spans).",
            "No pages of output."
        )),
        Some(DivergenceClass::UmberRaisesNoError)
    );
}

#[test]
fn reclassify_does_not_disturb_an_unrelated_shape() {
    // A same-line word substitution: neither the file-paren shape nor
    // either side being a `! `-prefixed error.
    assert_eq!(
        reclassify_no_error_channel(&mismatch(
            "cm, mm, dd, cc, nd, nc, bp, or sp; but yours is a new one!",
            "cm, mm, dd, cc, bp, or sp; but yours is a new one!"
        )),
        None
    );
}

#[test]
fn file_paren_closed_early_is_distinguished_from_no_error() {
    // conditionals/predicate-dispatch, real committed evidence.
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "(./predicate-dispatch.tex",
            "(./predicate-dispatch.tex)"
        ),
        Some(DivergenceClass::FileParenClosedEarly)
    );
    // main-control/mode-dispatch: the closing side carries extra content too.
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "(./mode-dispatch.tex",
            "(./mode-dispatch.tex [1.1.1.1.1.1] )"
        ),
        Some(DivergenceClass::FileParenClosedEarly)
    );
}

#[test]
fn true_no_error_is_still_recognized() {
    // alignments/span-width-record: oracle raises "! This can't happen
    // (256 spans)." and Umber's channel has no error anywhere.
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "! This can't happen (256 spans).\n",
            "No pages of output.\n"
        ),
        Some(DivergenceClass::UmberRaisesNoError)
    );
}

#[test]
fn different_error_is_not_mislabeled_no_error() {
    // input-expansion/input-outer-recovery: both sides raise an error, but a
    // different one -- the umber2-alfh.13 label is wrong for this shape.
    assert_eq!(
        classify(
            StreamChannel::Log,
            "Runaway definition?\n! Forbidden control sequence found.\n",
            "! Too many }'s.\n"
        ),
        Some(DivergenceClass::UmberRaisesUnexpectedError)
    );
}

#[test]
fn umber_erroring_where_oracle_does_not_is_the_opposite_of_no_error() {
    // math/head-for-vmode-recovery: the oracle channel is empty; Umber
    // raises an error the oracle never does.
    assert_eq!(
        classify(StreamChannel::Terminal, "", "! Missing { inserted.\n"),
        Some(DivergenceClass::UmberRaisesUnexpectedError)
    );
}

#[test]
fn terminal_prompt_omission_is_recognized() {
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "*\n",
            "*(Please type a command or say `\\end')\n"
        ),
        Some(DivergenceClass::TerminalPromptOmitted)
    );
}

#[test]
fn show_context_header_is_recognized() {
    assert_eq!(
        classify(
            StreamChannel::Log,
            "l.5 \\foo\n",
            "! Undefined control sequence.\n"
        ),
        Some(DivergenceClass::ShowContextAccuracy)
    );
    assert_eq!(
        classify(StreamChannel::Log, "<recently read> \\foo\n", "next\n"),
        Some(DivergenceClass::ShowContextAccuracy)
    );
    assert_eq!(
        classify(StreamChannel::Log, "\\foo ->bar\n", "next\n"),
        Some(DivergenceClass::ShowContextAccuracy)
    );
}

#[test]
fn box_diagnostic_is_recognized() {
    assert_eq!(
        classify(
            StreamChannel::Log,
            "Underfull \\hbox (badness 10000) in paragraph at lines 1--2\n",
            ")\n"
        ),
        Some(DivergenceClass::BoxDiagnostics)
    );
}

#[test]
fn etex_loaded_framing_is_recognized() {
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "This is pdfTeX, Version 3.141592653-2.6-1.40.29\n",
            ""
        ),
        Some(DivergenceClass::EtexLoadedFraming)
    );
}

#[test]
fn diagnostic_never_wraps_is_recognized() {
    assert_eq!(
        classify(
            StreamChannel::Terminal,
            "! Incompatible magnification (1300);\nthe previous value will be retained (1200).\n",
            "! Incompatible magnification (1300); the previous value will be retained (1200).\n"
        ),
        Some(DivergenceClass::DiagnosticNeverWraps)
    );
}

/// The `dvi` channel is deliberately never classified. It once had a single
/// byte-marker rule for the preamble banner comment, which
/// `channels::normalize_channel` now neutralizes on both sides, so a `dvi`
/// divergence that reaches a classifier at all is an individual defect with
/// no shared shape -- and guessing a bug id for it would misattribute a real
/// one. The banner bytes below are exactly what that retired rule matched.
#[test]
fn dvi_is_never_classified_even_for_the_retired_banner_shape() {
    let oracle = b"\x02\x00\x00\x00\x00xxx TeX output 2026.07.09:1336xxx";
    let umber = b"\x02\x00\x00\x00\x00xxx  Umber DVI 1970.01.01:0000xxx";
    assert_eq!(classify_divergence(StreamChannel::Dvi, oracle, umber), None);
}

#[test]
fn unclassifiable_shape_refuses_rather_than_guesses() {
    // scanners-internal-quantities/vacuous-dimension-units (umber2-alfh.20):
    // a same-line word substitution inside an ordinary sentence, not any
    // known shape -- must not be forced into one of the generic classes.
    assert_eq!(
        classify(
            StreamChannel::Log,
            "cm, mm, dd, cc, nd, nc, bp, or sp; but yours is a new one!\n",
            "cm, mm, dd, cc, bp, or sp; but yours is a new one!\n"
        ),
        None
    );
}

#[test]
fn every_class_names_a_valid_bug_id() {
    for class in [
        DivergenceClass::EtexLoadedFraming,
        DivergenceClass::ShowContextAccuracy,
        DivergenceClass::BoxDiagnostics,
        DivergenceClass::DiagnosticNeverWraps,
        DivergenceClass::FileParenClosedEarly,
        DivergenceClass::TerminalPromptOmitted,
        DivergenceClass::UmberRaisesNoError,
        DivergenceClass::UmberRaisesUnexpectedError,
    ] {
        assert!(super::super::valid_bug_id(class.bug()), "{:?}", class);
    }
}
