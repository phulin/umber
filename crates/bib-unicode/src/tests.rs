use super::*;

#[test]
fn pinned_identity_is_complete() {
    let version = UnicodeData::pinned().compatibility();
    assert_eq!(
        version.upstream_commit,
        "74252e608e5f8115375c532eb25416430a9f52eb"
    );
    assert_eq!(CollationData.table_id(), "biber-2.22/ducet-14.0.0");
}

#[test]
fn malformed_inputs_are_bounded() {
    assert_eq!(
        LanguageTag::parse(&"a".repeat(256)),
        Err(LanguageTagError::TooLong)
    );
    assert_eq!(
        ExtendedDate::parse("1996-13-03"),
        Err(DateError::InvalidMonth)
    );
    assert_eq!(
        ExtendedDate::parse(&"1".repeat(257)),
        Err(DateError::TooLong)
    );
    assert!(decode_legacy(&[0xff], LegacyEncoding::Utf8).is_err());
}

#[test]
fn compatibility_hash_is_md5() {
    assert_eq!(compatibility_hash("L"), "d20caec3b48a1eef164cb4ca81ba2587");
}

#[test]
fn name_hash_normalization_preserves_unicode_letters_and_tex_vestiges() {
    assert_eq!(normalise_string_hash("Š. Smith"), "ŠSmith");
    assert_eq!(normalise_string_hash(r#"Ä.~{\c{C}}.~{\c S}."#), "Äc:Cc:S");
}
