use tex_state::print::ErrorContextWidths;

use super::clipped_context;

#[test]
fn cropped_pseudoprint_preserves_the_location_label() {
    let widths = ErrorContextWidths::new(79, 35).expect("TeX82 context widths");
    let output = clipped_context(
        "l.26 ",
        r#"  \nonstopmode\lccode256-0\mathchardef\a="8000"#,
        r"\def\a{ SCALED 3~2769}",
        widths,
    );

    let mut lines = output.lines().skip(1);
    assert_eq!(lines.next(), Some("l.26 ...de256-0\\mathchardef\\a=\"8000"));
    let second = lines.next().expect("second context line");
    assert_eq!(second.chars().take(35).collect::<String>(), " ".repeat(35));
    assert_eq!(second.trim_start(), r"\def\a{ SCALED 3~2769}");
}
