use super::{
    CharacterCode, CharacterCodeError, CharacterMode, CommandDialect, CommandProfile,
    CommandProfileEncodingError,
};

#[test]
fn every_byte_roundtrips_without_becoming_a_unicode_scalar() {
    for byte in u8::MIN..=u8::MAX {
        let code = CharacterCode::from_byte(byte);
        assert!(code.is_byte());
        assert!(!code.is_unicode_scalar());
        assert_eq!(code.to_byte(), Ok(byte));
        assert_eq!(
            code.to_unicode_scalar(),
            Err(CharacterCodeError::ExpectedUnicodeScalar)
        );
        assert_eq!(
            CharacterCode::from_stable_bytes(code.to_stable_bytes()),
            Ok(code)
        );
    }
}

#[test]
fn valid_unicode_scalars_roundtrip_without_becoming_bytes() {
    for scalar in [0, 1, 0x41, 0xff, 0x100, 0xd7ff, 0xe000, 0x10ffff] {
        let code = CharacterCode::from_unicode_scalar(scalar).expect("valid scalar");
        assert!(code.is_unicode_scalar());
        assert!(!code.is_byte());
        assert_eq!(code.to_unicode_scalar(), Ok(scalar));
        assert_eq!(code.to_char().map(u32::from), Ok(scalar));
        assert_eq!(code.to_byte(), Err(CharacterCodeError::ExpectedByte));
        assert_eq!(
            CharacterCode::from_stable_bytes(code.to_stable_bytes()),
            Ok(code)
        );
    }

    assert_ne!(CharacterCode::from_byte(b'A'), CharacterCode::from('A'));
}

#[test]
fn every_unicode_scalar_has_one_canonical_stable_encoding() {
    for scalar in 0..=0x10_ffff {
        let result = CharacterCode::from_unicode_scalar(scalar);
        if (0xd800..=0xdfff).contains(&scalar) {
            assert_eq!(
                result,
                Err(CharacterCodeError::InvalidUnicodeScalar(scalar))
            );
            continue;
        }

        let code = result.expect("every nonsurrogate code point is a scalar");
        assert_eq!(code.to_unicode_scalar(), Ok(scalar));
        assert_eq!(
            CharacterCode::from_stable_bytes(code.to_stable_bytes()),
            Ok(code)
        );
    }
}

#[test]
fn invalid_scalars_and_noncanonical_encodings_are_rejected() {
    for scalar in [0xd800, 0xdfff, 0x11_0000, u32::MAX] {
        assert_eq!(
            CharacterCode::from_unicode_scalar(scalar),
            Err(CharacterCodeError::InvalidUnicodeScalar(scalar))
        );
    }
    assert_eq!(
        CharacterCode::from_stable_bytes(256_u32.to_le_bytes()),
        Err(CharacterCodeError::InvalidEncoding(256))
    );
    let tagged_surrogate = (0x8000_0000_u32 | 0xd800).to_le_bytes();
    assert_eq!(
        CharacterCode::from_stable_bytes(tagged_surrogate),
        Err(CharacterCodeError::InvalidEncoding(
            0x8000_0000_u32 | 0xd800
        ))
    );
}

#[test]
fn canonical_and_unicode_profiles_are_immutable_distinct_identities() {
    let profiles = [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
        CommandProfile::unicode_extended(CommandDialect::Tex82),
        CommandProfile::unicode_extended(CommandDialect::Etex26),
        CommandProfile::unicode_extended(CommandDialect::Pdftex14029),
    ];

    for profile in profiles {
        assert_eq!(
            CommandProfile::from_stable_bytes(profile.to_stable_bytes()),
            Ok(profile)
        );
        assert_eq!(profile.fingerprint(), profile.fingerprint());
    }
    for (index, profile) in profiles.iter().enumerate() {
        assert!(
            profiles[index + 1..]
                .iter()
                .all(|other| profile.fingerprint() != other.fingerprint())
        );
    }

    assert_eq!(
        CommandProfile::TEX82.character_mode(),
        CharacterMode::EightBitExact
    );
    assert_eq!(
        CommandProfile::unicode_extended(CommandDialect::Tex82).character_mode(),
        CharacterMode::UnicodeExtended
    );

    assert_eq!(profiles[0].fingerprint().get(), 0x6a66_4a3d_7375_1f34);
    assert_eq!(profiles[1].fingerprint().get(), 0x730f_c43d_785d_1c2c);
    assert_eq!(profiles[2].fingerprint().get(), 0x7bb9_3f3d_7d45_1ad7);
    assert_eq!(profiles[3].fingerprint().get(), 0x6a69_ac3d_7377_fb91);
    assert_eq!(profiles[4].fingerprint().get(), 0x730c_643d_785a_4335);
    assert_eq!(profiles[5].fingerprint().get(), 0x7bbc_ab3d_7d48_0832);
}

#[test]
fn output_encoding_policy_distinguishes_exact_bytes_from_unicode() {
    let exact = CommandProfile::TEX82;
    let unicode = CommandProfile::unicode_extended(CommandDialect::Tex82);
    let byte_codes = [0x00, 0x0f, 0x7f, 0xff].map(CharacterCode::from_byte);
    let exact_bytes = byte_codes
        .into_iter()
        .flat_map(|code| exact.encode_output_character(code).expect("exact byte"))
        .collect::<Vec<_>>();

    assert_eq!(exact_bytes, [0x00, 0x0f, 0x7f, 0xff]);
    assert_eq!(
        unicode.encode_output_character(
            CharacterCode::from_unicode_scalar(0xff).expect("U+00FF is a scalar")
        ),
        Ok(vec![0xc3, 0xbf])
    );
    assert_eq!(
        unicode.encode_output_character(CharacterCode::from_byte(0xff)),
        Err(CharacterCodeError::ExpectedUnicodeScalar)
    );
    assert_eq!(
        exact.encode_output_character(
            CharacterCode::from_unicode_scalar(0xff).expect("U+00FF is a scalar")
        ),
        Err(CharacterCodeError::ExpectedByte)
    );
}

#[test]
fn output_text_reapplies_the_profile_domain_after_token_display() {
    let exact = CommandProfile::TEX82;
    let unicode = CommandProfile::unicode_extended(CommandDialect::Tex82);

    // Exact-byte token display projects 0xC3 and 0xA3 through their
    // corresponding Rust scalars; the output boundary must not UTF-8 encode
    // those scalar projections a second time.
    assert_eq!(
        exact.encode_output_text("MagalhÃ£es"),
        Ok(b"Magalh\xc3\xa3es".to_vec())
    );
    assert_eq!(
        unicode.encode_output_text("Magalhães"),
        Ok(b"Magalh\xc3\xa3es".to_vec())
    );
    assert_eq!(
        exact.encode_output_text("outside-\u{100}"),
        Err(CharacterCodeError::ExpectedByte)
    );
}

#[test]
fn capabilities_are_semantic_and_validated_during_decode() {
    let tex = CommandProfile::TEX82.capabilities();
    assert!(!tex.supports_etex());
    assert!(!tex.supports_pdftex());
    assert!(!tex.supports_unicode());

    let etex_unicode = CommandProfile::unicode_extended(CommandDialect::Etex26).capabilities();
    assert!(etex_unicode.supports_etex());
    assert!(!etex_unicode.supports_pdftex());
    assert!(etex_unicode.supports_unicode());

    let pdftex = CommandProfile::PDFTEX14029.capabilities();
    assert!(pdftex.supports_etex());
    assert!(pdftex.supports_pdftex());
    assert!(!pdftex.supports_unicode());

    let mut invalid = CommandProfile::PDFTEX14029.to_stable_bytes();
    invalid[3] = 0;
    assert_eq!(
        CommandProfile::from_stable_bytes(invalid),
        Err(CommandProfileEncodingError::CapabilityMismatch {
            expected: 0b011,
            found: 0,
        })
    );
}

#[test]
fn profile_decode_rejects_every_unrecognized_identity_component() {
    let mut unsupported_version = CommandProfile::TEX82.to_stable_bytes();
    unsupported_version[0] = 2;
    assert_eq!(
        CommandProfile::from_stable_bytes(unsupported_version),
        Err(CommandProfileEncodingError::UnsupportedVersion(2))
    );

    let mut unknown_dialect = CommandProfile::TEX82.to_stable_bytes();
    unknown_dialect[1] = 3;
    assert_eq!(
        CommandProfile::from_stable_bytes(unknown_dialect),
        Err(CommandProfileEncodingError::UnknownDialect(3))
    );

    let mut unknown_mode = CommandProfile::TEX82.to_stable_bytes();
    unknown_mode[2] = 2;
    assert_eq!(
        CommandProfile::from_stable_bytes(unknown_mode),
        Err(CommandProfileEncodingError::UnknownCharacterMode(2))
    );
}
