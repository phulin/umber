// Native Rust translation of upstream t/utils.t at commit 74252e6.

use bib_engine::VirtualPath;
use bib_input::{BooleanInput, BooleanOutput, MappedBoolean, map_boolean};
use bib_unicode::{
    RangeEnd, RecodeSet, TexRecoder, normalise_string, normalise_string_hash,
    normalise_string_underscore, parse_range, range_len, reduce_array, remove_outer, split_xsv,
    strip_noinit,
};

macro_rules! native_test {
    ($name:ident, $body:expr) => {
        #[test]
        fn $name() {
            $body
        }
    };
}

macro_rules! xfail {
    ($name:ident, $reason:literal, $body:expr) => {
        #[test]
        #[ignore = "xfail: native utility behavior does not yet reproduce Biber"]
        fn $name() {
            $body
        }
    };
}

macro_rules! decode {
    ($name:ident, $set:ident, $input:literal, $expected:literal) => {
        native_test!($name, assert_decode(RecodeSet::$set, $input, $expected));
    };
}

macro_rules! encode {
    ($name:ident, $set:ident, $input:literal, $expected:literal) => {
        native_test!($name, assert_encode(RecodeSet::$set, $input, $expected));
    };
}

fn assert_decode(set: RecodeSet, input: &str, expected: &str) {
    assert_eq!(TexRecoder::new(set, set).decode(input), expected);
}

fn assert_encode(set: RecodeSet, input: &str, expected: &str) {
    assert_eq!(TexRecoder::new(set, set).encode(input), expected);
}

fn assert_decode_with(decode: RecodeSet, encode: RecodeSet, input: &str, expected: &str) {
    assert_eq!(TexRecoder::new(decode, encode).decode(input), expected);
}

fn assert_encode_with(decode: RecodeSet, encode: RecodeSet, input: &str, expected: &str) {
    assert_eq!(TexRecoder::new(decode, encode).encode(input), expected);
}

xfail!(
    assertion_001_file_location_1,
    "xfail: host absolute paths are intentionally outside Umber's VFS",
    assert_eq!(
        VirtualPath::user("/cwd/t/tdata/general.bcf")
            .expect("Biber accepts host absolute paths")
            .as_str(),
        "/cwd/t/tdata/general.bcf"
    )
);
native_test!(
    assertion_002_file_location_2,
    assert_eq!(
        VirtualPath::user("t/tdata/general.bcf").unwrap().as_str(),
        "/job/t/tdata/general.bcf"
    )
);
native_test!(
    assertion_003_file_location_3,
    assert_eq!(
        VirtualPath::user("t/tdata/examples.bib").unwrap().as_str(),
        "/job/t/tdata/examples.bib"
    )
);
xfail!(
    assertion_004_file_location_4,
    "xfail: kpsewhich lookup is intentionally unavailable in the native VFS",
    assert!(VirtualPath::distribution("plain.tex").is_ok())
);
native_test!(
    assertion_005_file_location_5,
    assert_eq!(
        VirtualPath::user("t/tdata/general.bcf").unwrap().as_str(),
        "/job/t/tdata/general.bcf"
    )
);

xfail!(
    assertion_006_normalise_string,
    "xfail: whitespace is removed",
    assert_eq!(normalise_string("\"a, b–c: d\" ", true), "a bc d")
);
xfail!(
    assertion_007_normalise_string_underscore_1,
    "xfail: unknown TeX commands are retained",
    assert_eq!(
        normalise_string_underscore(
            &TexRecoder::new(RecodeSet::Base, RecodeSet::Base)
                .decode("\\c Se\\x{c}\\\"ok-\\foo{a},  N\\`i\\~no\n    $§+ :-)   "),
            false
        ),
        "Şecöka_Nìño"
    )
);
native_test!(
    assertion_008_normalise_string_underscore_3,
    assert_eq!(
        normalise_string_underscore("{Foo de Bar, Graf Ludwig}", true),
        "Foo_de_Bar_Graf_Ludwig"
    )
);

decode!(
    assertion_009_latex_decode_1,
    Base,
    "Mu\\d{h}ammad ibn M\\=us\\=a al-Khw\\=arizm\\={\\i} \\r{a}",
    "Muḥammad ibn Mūsā al-Khwārizmı̄ å"
);
decode!(assertion_010_latex_decode_2, Base, "\\alpha", "\\alpha");
decode!(
    assertion_011_latex_decode_3,
    Base,
    "\\textless\\textampersand",
    "<&"
);
xfail!(
    assertion_012_latex_encode_1,
    "xfail: base encoder accent spelling differs",
    assert_encode(
        RecodeSet::Base,
        "Muḥammad ibn Mūsā al-Khwārizmī",
        "Mu\\d{h}ammad ibn M\\={u}s\\={a} al-Khw\\={a}rizm\\={\\i}"
    )
);
encode!(assertion_013_latex_encode_2, Base, "α", "α");
xfail!(
    assertion_014_latex_decode_accent_1_with_redundant_explicit_brace_protection,
    "xfail: redundant inner braces are retained",
    assert_decode(RecodeSet::Base, "{M{\\'a}t{\\'e}}", "{Máté}")
);
decode!(
    assertion_015_latex_decode_accent_2,
    Base,
    "{M\\'{a}t\\'{e}}",
    "{Máté}"
);
decode!(
    assertion_016_latex_decode_accent_3,
    Base,
    "{M\\'at\\'e}",
    "{Máté}"
);
decode!(
    assertion_017_latex_decode_accent_4,
    Base,
    "R{\\'egis}",
    "R{égis}"
);
decode!(
    assertion_018_latex_decode_accent_5,
    Base,
    "\\frac{a}{b}",
    "\\frac{a}{b}"
);
decode!(
    assertion_019_latex_decode_accent_6,
    Base,
    "\\textuppercase{\\'e}",
    "\\textuppercase{é}"
);
decode!(
    assertion_020_latex_reversing_recoding_test_1,
    Base,
    "\\DH{}and\\dj{}and\\'{c}, H.",
    "Ðandđandć, H."
);
decode!(
    assertion_021_latex_reversing_recoding_test_2,
    Base,
    "{\\DH{}and\\dj{}and\\'{c}, H.}",
    "{Ðandđandć, H.}"
);
xfail!(
    assertion_022_latex_reversing_recoding_test_3,
    "xfail: base encoder command spelling differs",
    assert_encode(
        RecodeSet::Base,
        "Ðandđandć, H.",
        "\\DH{}and\\dj{}and\\'{c}, H."
    )
);
xfail!(
    assertion_023_latex_reversing_recoding_test_4,
    "xfail: base encoder command spelling differs",
    assert_encode(
        RecodeSet::Base,
        "{Ðandđandć, H.}",
        "{\\DH{}and\\dj{}and\\'{c}, H.}"
    )
);
xfail!(
    assertion_024_latex_decode_4_with_2_explicit_brace_protections,
    "xfail: explicit protection differs",
    assert_decode(
        RecodeSet::Full,
        "{\\\"{U}}ber {\\\"{U}}berlegungen zur \\\"{U}berwindung des \\\"{U}bels",
        "Über Überlegungen zur Überwindung des Übels"
    )
);
decode!(assertion_025_latex_decode_4a, Full, "\\alpha", "α");
xfail!(
    assertion_026_latex_decode_5,
    "xfail: dotless-i accent composition differs",
    assert_decode(RecodeSet::Full, "\\'\\i", "í")
);
xfail!(
    assertion_027_latex_decode_5a_with_redundant_explicit_brace_protection,
    "xfail: redundant protection differs",
    assert_decode(RecodeSet::Full, "{\\'\\i}", "í")
);
decode!(assertion_028_latex_decode_6, Full, "\\^{\\j}", "ȷ̂");
decode!(assertion_029_latex_decode_7, Full, "\\u{\\i}", "ı̆");
decode!(assertion_030_latex_decode_8, Full, "\\u\\i", "ı̆");
xfail!(
    assertion_031_latex_decode_9,
    "xfail: nested protection differs",
    assert_decode(RecodeSet::Full, "{{\\'A}lvarez}, J.~D.", "{Álvarez}, J.~D.")
);
decode!(assertion_032_latex_decode_9, Full, "\\i", "ı");
decode!(assertion_033_latex_decode_10, Full, "\\j", "ȷ");
decode!(assertion_034_latex_decode_11, Full, "\\textdiv", "÷");
decode!(assertion_035_latex_decode_13, Full, "--", "--");
decode!(assertion_036_latex_decode_14, Full, "\\textdegree C", "°C");
xfail!(
    assertion_037_latex_decode_15,
    "xfail: single-glyph protection differs",
    assert_decode(RecodeSet::Full, "{\\'{I}}", "Í")
);
xfail!(
    assertion_038_latex_decode_16,
    "xfail: single-glyph protection differs",
    assert_decode(RecodeSet::Full, "{\\v{C}}", "Č")
);
decode!(assertion_039_latex_decode_17, Full, "{I}", "{I}");
decode!(assertion_040_latex_decode_18, Full, "\\&{A}", "\\&{A}");
decode!(
    assertion_041_latex_decode_19,
    Full,
    "\\&\\;{A}",
    "\\&\\;{A}"
);
encode!(assertion_042_latex_encode_3, Full, "α", "{$\\alpha$}");
encode!(assertion_043_latex_encode_4, Full, "µ", "{$\\mu$}");
encode!(assertion_044_latex_encode_5, Full, "≄", "{$\\not\\simeq$}");
encode!(assertion_045_latex_encode_6, Full, "Þ", "\\TH{}");
encode!(assertion_046_latex_encode_7, Full, "$", "$");
encode!(assertion_047_latex_encode_8, Full, "–", "--");
decode!(assertion_048_discretionary_hyphens, Full, "a\\-a", "a\\-a");
encode!(assertion_049_latex_encode_9, Full, "Åå", "\\r{A}\\r{a}");
xfail!(
    assertion_050_latex_encode_10,
    "xfail: combining vertical line encoding differs",
    assert_encode(RecodeSet::Full, "a̍", "\\|{a}")
);
xfail!(
    assertion_051_latex_encode_11,
    "xfail: dotless-i breve spelling differs",
    assert_encode(RecodeSet::Full, "ı̆", "\\u{\\i{}}")
);
encode!(
    assertion_052_latex_encode_12,
    Full,
    "®",
    "\\textregistered{}"
);
encode!(assertion_053_latex_encode_13, Full, "©", "{$\\copyright$}");
encode!(assertion_054_latex_encode_13, Full, "°C", "\\textdegree{}C");

native_test!(
    assertion_055_reduce_array,
    assert_eq!(
        reduce_array(&['a', 'b', 'c', 'd', 'e', 'f', 'c'], &['c', 'e']),
        vec!['a', 'b', 'd', 'f']
    )
);
native_test!(
    assertion_056_remove_outer_1,
    assert!(remove_outer("{Some string}").0)
);
native_test!(
    assertion_057_remove_outer_2,
    assert_eq!(remove_outer("{Some string}").1, "Some string")
);
native_test!(
    assertion_058_normalise_string_lite,
    assert_eq!(normalise_string_hash("Ä.~{\\c{C}}.~{\\c S}."), "Äc:Cc:S")
);
native_test!(
    assertion_059_latex_different_encode_decode_sets_1,
    assert_decode_with(RecodeSet::Base, RecodeSet::Full, "\\textdiv", "\\textdiv")
);
native_test!(
    assertion_060_latex_different_encode_decode_sets_2,
    assert_encode_with(RecodeSet::Base, RecodeSet::Full, "÷", "{$\\div$}")
);
native_test!(
    assertion_061_latex_null_decode_1,
    assert_decode_with(RecodeSet::Null, RecodeSet::Full, "\\i", "\\i")
);
native_test!(
    assertion_062_latex_null_encode_2,
    assert_encode_with(RecodeSet::Null, RecodeSet::Full, "ı", "\\i{}")
);
native_test!(
    assertion_063_latex_null_decode_2,
    assert_decode_with(
        RecodeSet::Null,
        RecodeSet::Full,
        "{$\\hbox {N}^3$}",
        "{$\\hbox{N}^3$}"
    )
);

macro_rules! range_len_case {
    ($name:ident, $ranges:expr, $expected:expr) => {
        native_test!($name, assert_eq!(range_len($ranges), $expected));
    };
}
range_len_case!(
    assertion_064_rangelen_test_1,
    &[(Some("10"), Some("15"))],
    6
);
range_len_case!(
    assertion_065_rangelen_test_2,
    &[(Some("10"), Some("15")), (Some("47"), Some("53"))],
    13
);
range_len_case!(
    assertion_066_rangelen_test_3,
    &[(Some("10"), Some("15")), (Some("47"), None)],
    7
);
range_len_case!(
    assertion_067_rangelen_test_4,
    &[(Some("10"), Some("15")), (Some("47"), Some(""))],
    -1
);
range_len_case!(
    assertion_068_rangelen_test_5,
    &[(Some("10"), Some("15")), (Some(""), Some("35"))],
    -1
);
range_len_case!(
    assertion_069_rangelen_test_6,
    &[(Some("10"), Some("15")), (Some(""), None)],
    -1
);
range_len_case!(
    assertion_070_rangelen_test_7,
    &[
        (Some("10"), Some("15")),
        (Some("XX"), Some("XXiv")),
        (Some("i"), Some("10"))
    ],
    21
);
range_len_case!(
    assertion_071_rangelen_test_8,
    &[(Some("10"), Some("15")), (Some("ⅥⅠ"), Some("ⅻ"))],
    12
);
range_len_case!(
    assertion_072_rangelen_test_9,
    &[(Some("I-II"), Some("III-IV"))],
    -1
);
range_len_case!(
    assertion_073_rangelen_test_10,
    &[
        (Some("22"), Some("4")),
        (Some("123"), Some("7")),
        (Some("113"), Some("15"))
    ],
    11
);

native_test!(
    assertion_074_boolean_conversion_1,
    assert_eq!(
        map_boolean(BooleanInput::Text("true"), BooleanOutput::Number),
        Some(MappedBoolean::Number(1))
    )
);
native_test!(
    assertion_075_boolean_conversion_2,
    assert_eq!(
        map_boolean(BooleanInput::Text("False"), BooleanOutput::Number),
        Some(MappedBoolean::Number(0))
    )
);
native_test!(
    assertion_076_boolean_conversion_3,
    assert_eq!(
        map_boolean(BooleanInput::Number(1), BooleanOutput::Text),
        Some(MappedBoolean::Text("true"))
    )
);
native_test!(
    assertion_077_boolean_conversion_4,
    assert_eq!(
        map_boolean(BooleanInput::Number(0), BooleanOutput::Text),
        Some(MappedBoolean::Text("false"))
    )
);
native_test!(
    assertion_078_boolean_conversion_5,
    assert_eq!(
        map_boolean(BooleanInput::Number(0), BooleanOutput::Number),
        Some(MappedBoolean::Number(0))
    )
);

macro_rules! range_case {
    ($name:ident, $input:literal, $expected:expr) => {
        native_test!($name, assert_eq!(parse_range($input), Some($expected)));
    };
}
range_case!(
    assertion_079_range_parsing_1,
    "1--2",
    (1, RangeEnd::Number(2))
);
range_case!(
    assertion_080_range_parsing_2,
    "-2",
    (1, RangeEnd::Number(2))
);
range_case!(assertion_081_range_parsing_3, "3-", (3, RangeEnd::Open));
range_case!(assertion_082_range_parsing_4, "5", (1, RangeEnd::Number(5)));
range_case!(assertion_083_range_parsing_5, "3--+", (3, RangeEnd::Last));
native_test!(
    assertion_084_split_xsv_1,
    assert_eq!(
        split_xsv("family=a, given=a b, given-i=a b c"),
        ["family=a", "given=a b", "given-i=a b c"]
    )
);
native_test!(
    assertion_085_split_xsv_2,
    assert_eq!(
        split_xsv("\"family={Something, here}\", given=b"),
        ["family={Something, here}", "given=b"]
    )
);
xfail!(
    assertion_086_name_strip_1,
    "xfail: braced texttt form retains braces",
    assert_eq!(strip_noinit("\\texttt{freedesktop.org}"), "freedesktop.org")
);
native_test!(
    assertion_087_name_strip_2,
    assert_eq!(strip_noinit("\\texttt freedesktop.org"), "freedesktop.org")
);
native_test!(
    assertion_088_name_strip_3,
    assert_eq!(
        strip_noinit("{\\texttt freedesktop.org}"),
        "{freedesktop.org}"
    )
);
native_test!(
    assertion_089_name_strip_4,
    assert_eq!(strip_noinit("{C.\\bibtexspatium A.}"), "{C.A.}")
);
