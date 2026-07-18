use super::*;

#[test]
fn header_and_directory_are_fixed_width_little_endian_and_canonical() {
    let encoded = encode(&[
        SectionInput {
            kind: 9,
            alignment: 32,
            bytes: b"second",
        },
        SectionInput {
            kind: 2,
            alignment: 8,
            bytes: b"first",
        },
    ])
    .expect("encode container");

    assert_eq!(&encoded[..8], &MAGIC);
    assert_eq!(read_u32(&encoded, 8), SCHEMA_VERSION);
    assert_eq!(read_u32(&encoded, 12), HEADER_LEN as u32);
    assert_eq!(read_u32(&encoded, 16), DIRECTORY_ENTRY_LEN as u32);
    assert_eq!(read_u32(&encoded, 20), 2);
    assert_eq!(read_u64(&encoded, 24), HEADER_LEN as u64);
    assert_eq!(read_u64(&encoded, 32), encoded.len() as u64);
    assert_eq!(read_u64(&encoded, 40), ABI_FINGERPRINT);
    assert_eq!(read_u64(&encoded, 48), LOOKUP_CONFIGURATION_FINGERPRINT);

    let decoded = decode(&encoded).expect("decode container");
    assert_eq!(decoded.sections.len(), 2);
    assert_eq!(decoded.sections[0].kind, 2);
    assert_eq!(decoded.sections[0].alignment, 8);
    assert_eq!(decoded.sections[0].bytes, b"first");
    assert_eq!(decoded.sections[1].kind, 9);
    assert_eq!(decoded.sections[1].alignment, 32);
    assert_eq!(decoded.sections[1].bytes, b"second");
    assert!(
        encoded[decoded.sections[0].offset + 5..decoded.sections[1].offset]
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn checksum_covers_header_directory_padding_and_payload() {
    let encoded = encode(&[SectionInput {
        kind: 1,
        alignment: 64,
        bytes: b"payload",
    }])
    .expect("encode container");
    let section = decode(&encoded).expect("decode container").sections[0];
    for offset in [40, HEADER_LEN + 8, section.offset - 1, section.offset] {
        let mut corrupted = encoded.clone();
        corrupted[offset] ^= 1;
        assert_eq!(decode(&corrupted), Err(ContainerError::Checksum));
    }
}

#[test]
fn authoritative_fingerprints_and_structure_fail_closed() {
    let encoded = encode(&[SectionInput {
        kind: 1,
        alignment: 8,
        bytes: b"payload",
    }])
    .expect("encode container");

    let mut bad_abi = encoded.clone();
    bad_abi[40] ^= 1;
    refresh_checksum(&mut bad_abi);
    assert!(matches!(
        decode(&bad_abi),
        Err(ContainerError::IncompatibleAbi(_))
    ));

    let mut bad_configuration = encoded.clone();
    bad_configuration[48] ^= 1;
    refresh_checksum(&mut bad_configuration);
    assert!(matches!(
        decode(&bad_configuration),
        Err(ContainerError::IncompatibleLookupConfiguration(_))
    ));

    let mut bad_offset = encoded;
    bad_offset[HEADER_LEN + 8..HEADER_LEN + 16].copy_from_slice(&0_u64.to_le_bytes());
    refresh_checksum(&mut bad_offset);
    assert_eq!(
        decode(&bad_offset),
        Err(ContainerError::Invalid("non-canonical section offset"))
    );
}

#[test]
fn codec_rejects_native_shaped_or_ambiguous_geometry() {
    assert_eq!(
        encode(&[SectionInput {
            kind: 1,
            alignment: 3,
            bytes: b"x",
        }]),
        Err(ContainerError::Invalid("invalid section alignment"))
    );
    assert_eq!(
        encode(&[
            SectionInput {
                kind: 1,
                alignment: 8,
                bytes: b"x",
            },
            SectionInput {
                kind: 1,
                alignment: 8,
                bytes: b"y",
            },
        ]),
        Err(ContainerError::Invalid("duplicate or zero section kind"))
    );
}
