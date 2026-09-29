//! Batched `\csname` character runs against the scalar observed scan.

use super::*;

/// Runs `source` unobserved (batched resident runs) and observed (scalar
/// delivery only), returning each run's terminal text and `\count1`–`\count3`.
fn csname_parity(source: &[u8]) -> (String, [i32; 3]) {
    let run = |observed: bool| {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            if observed {
                let mut observations = ObservationRecorder::default();
                run_to_end_observed(&mut control, stores, &mut observations);
            } else {
                run_to_end(&mut control, stores);
            }
            let count = |index| stores.count(index).expect("count register");
            (terminal_text(stores), [count(1), count(2), count(3)])
        })
    };
    let unobserved = run(false);
    assert_eq!(run(true), unobserved);
    unobserved
}

#[test]
fn csname_runs_join_body_argument_and_nested_characters() {
    // Characters come from a macro body, a delimited and an undelimited
    // argument, a nested expansion, a space, and a doubled parameter
    // character; `\csname` accepts every non-active literal (TeX82 §372).
    let (terminal, counts) = csname_parity(
        br"\def\mid{b c##}\def\name#1.#2{\csname x#1\mid#2y\endcsname}
           \expandafter\countdef\csname x12b c#34y\endcsname=1
           \expandafter\countdef\csname x$^_\endcsname=2
           \def\math{$^_}
           \name12.{34}=5 \csname x\math\endcsname=6 \end",
    );
    assert_eq!(terminal, "");
    assert_eq!(counts, [5, 6, 0]);
}

#[test]
fn csname_runs_stop_before_braces_and_control_sequences() {
    // A brace is still a character of the name but stays with the scalar
    // scan, which owns `align_state`; `\relax` ends the name with §373's
    // recovery.
    let (terminal, counts) = csname_parity(
        br"\expandafter\countdef\csname a{b}c\endcsname=1
           \def\braced{a{b}c}\csname\braced\endcsname=7
           \def\bad{ab\relax}\expandafter\ifx\csname\bad\endcsname\relax\count3=1 \fi
           \count2=8 \end",
    );
    assert!(
        terminal.starts_with("! Missing \\endcsname inserted."),
        "{terminal}"
    );
    assert_eq!(counts, [7, 8, 1]);
}

#[test]
fn csname_runs_leave_noexpand_frames_to_scalar_settlement() {
    // `\noexpand` marks its frame, whose control sequence must reach the
    // scalar scan as the `\relax` meaning §369 gives it. The recovery then
    // backs up the undefined control sequence, which reports its own error.
    let (terminal, counts) = csname_parity(
        br"\def\x{ab\noexpand\undefined}\expandafter\let\expandafter\y\csname\x\endcsname
           \ifx\y\relax\count1=3 \fi\end",
    );
    assert!(
        terminal.starts_with("! Missing \\endcsname inserted."),
        "{terminal}"
    );
    assert_eq!(counts[0], 3);
}
