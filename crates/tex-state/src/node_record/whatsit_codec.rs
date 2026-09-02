use super::*;

fn append_bytes<Kind>(annex: &mut NodeAnnexWriter<'_>, bytes: &[u8]) -> AnnexKey<Kind> {
    let mut body = Vec::with_capacity(1 + bytes.len().div_ceil(4));
    body.push(u32::try_from(bytes.len()).expect("node annex byte length fits u32"));
    for chunk in bytes.chunks(4) {
        let mut packed = [0; 4];
        packed[..chunk.len()].copy_from_slice(chunk);
        body.push(u32::from_le_bytes(packed));
    }
    annex.append_span(&body)
}

fn detach_bytes<Kind>(annex: NodeAnnexView<'_>, key: AnnexKey<Kind>) -> Option<Vec<u8>> {
    let body = annex.detach_span(key)?;
    let len = *body.first()? as usize;
    if body.len() != 1 + len.div_ceil(4) {
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    for word in &body[1..] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.truncate(len);
    Some(bytes)
}

fn encode_identifier(identifier: NodePdfActionIdentifier<NodeTokenKey>) -> (u32, [u32; 6]) {
    match identifier {
        NodePdfActionIdentifier::Name(tokens) => (0, tokens.coordinates()),
        NodePdfActionIdentifier::Number(number) => (1, [number, 0, 0, 0, 0, 0]),
        NodePdfActionIdentifier::Raw(tokens) => (2, tokens.coordinates()),
    }
}

fn decode_identifier(tag: u32, words: [u32; 6]) -> Option<NodePdfActionIdentifier<NodeTokenKey>> {
    match tag {
        0 => Some(NodePdfActionIdentifier::Name(
            NodeTokenKey::from_coordinates(words),
        )),
        1 if words[1..].iter().all(|word| *word == 0) => {
            Some(NodePdfActionIdentifier::Number(words[0]))
        }
        2 => Some(NodePdfActionIdentifier::Raw(
            NodeTokenKey::from_coordinates(words),
        )),
        _ => None,
    }
}

fn encode_pdf_dimensions(value: crate::PdfAnnotationDimensions) -> ([u32; 3], u32) {
    (
        [
            value.width.map_or(0, scaled_word),
            value.height.map_or(0, scaled_word),
            value.depth.map_or(0, scaled_word),
        ],
        bool_word(value.width.is_some())
            | (bool_word(value.height.is_some()) << 1)
            | (bool_word(value.depth.is_some()) << 2),
    )
}

fn decode_pdf_dimensions(words: [u32; 3], presence: u32) -> Option<crate::PdfAnnotationDimensions> {
    if presence & !7 != 0 {
        return None;
    }
    Some(crate::PdfAnnotationDimensions {
        width: (presence & 1 != 0).then(|| decode_scaled(words[0])),
        height: (presence & 2 != 0).then(|| decode_scaled(words[1])),
        depth: (presence & 4 != 0).then(|| decode_scaled(words[2])),
    })
}

fn encode_destination_kind(value: PdfDestinationKind) -> (u32, [u32; 3], u32) {
    match value {
        PdfDestinationKind::Xyz { zoom } => (
            0,
            [zoom.unwrap_or_default() as u32, 0, 0],
            bool_word(zoom.is_some()),
        ),
        PdfDestinationKind::FitBoundingBoxHorizontal => (1, [0; 3], 0),
        PdfDestinationKind::FitBoundingBoxVertical => (2, [0; 3], 0),
        PdfDestinationKind::FitBoundingBox => (3, [0; 3], 0),
        PdfDestinationKind::FitHorizontal => (4, [0; 3], 0),
        PdfDestinationKind::FitVertical => (5, [0; 3], 0),
        PdfDestinationKind::FitRectangle(dimensions) => {
            let (words, presence) = encode_pdf_dimensions(dimensions);
            (6, words, presence)
        }
        PdfDestinationKind::Fit => (7, [0; 3], 0),
    }
}

fn decode_destination_kind(tag: u32, words: [u32; 3], presence: u32) -> Option<PdfDestinationKind> {
    match tag {
        0 if presence <= 1 && words[1] == 0 && words[2] == 0 => Some(PdfDestinationKind::Xyz {
            zoom: (presence == 1).then_some(words[0] as i32),
        }),
        1 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBoxHorizontal),
        2 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBoxVertical),
        3 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBox),
        4 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitHorizontal),
        5 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitVertical),
        6 => Some(PdfDestinationKind::FitRectangle(decode_pdf_dimensions(
            words, presence,
        )?)),
        7 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::Fit),
        _ => None,
    }
}

pub(super) fn encode_whatsit(value: Whatsit, annex: &mut NodeAnnexWriter<'_>) -> NodeRecord {
    match value {
        Whatsit::OpenOut { slot, path } => {
            let path = append_bytes::<Utf8Span>(annex, path.as_bytes());
            let mut body = Vec::with_capacity(8);
            append_words(&mut body, path.words());
            body.push(u32::from(slot.raw()));
            NodeRecord::with_key(
                NodeKind::Whatsit,
                0,
                0,
                annex.append_fixed::<OpenOutPayload>(&body),
            )
        }
        Whatsit::CloseOut { slot } => NodeRecord::new(
            NodeKind::Whatsit,
            1,
            bool_word(slot.is_some()),
            [
                slot.map_or(0, |slot| u32::from(slot.raw())),
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        ),
        Whatsit::DeferredWrite { sink, tokens } => {
            let key = tokens.coordinates();
            NodeRecord::new(
                NodeKind::Whatsit,
                2,
                0,
                [
                    key[0],
                    key[1],
                    key[2],
                    key[3],
                    key[4],
                    key[5],
                    encode_print_sink(sink),
                ],
            )
        }
        Whatsit::Special { class, payload } => {
            let class = append_bytes::<Utf8Span>(annex, class.as_bytes());
            let payload = append_bytes::<ByteSpan>(annex, &payload);
            let mut body = Vec::with_capacity(12);
            append_words(&mut body, class.words());
            append_words(&mut body, payload.words());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                3,
                0,
                annex.append_fixed::<SpecialPayload>(&body),
            )
        }
        Whatsit::DeferredSpecial { class, tokens } => {
            let class = append_bytes::<Utf8Span>(annex, class.as_bytes());
            let mut body = Vec::with_capacity(12);
            append_words(&mut body, class.words());
            append_words(&mut body, tokens.coordinates());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                4,
                0,
                annex.append_fixed::<DeferredSpecialPayload>(&body),
            )
        }
        Whatsit::PdfReferenceObject { object } => {
            NodeRecord::new(NodeKind::Whatsit, 5, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfAccessibility(value) => NodeRecord::new(
            NodeKind::Whatsit,
            6,
            match value {
                PdfAccessibilityControl::InterwordSpaceOn => 0,
                PdfAccessibilityControl::InterwordSpaceOff => 1,
                PdfAccessibilityControl::FakeSpace => 2,
            },
            [0; 7],
        ),
        Whatsit::PdfAnnotation { object } => {
            NodeRecord::new(NodeKind::Whatsit, 7, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfLinkStart { object } => {
            NodeRecord::new(NodeKind::Whatsit, 8, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfLinkEnd { object } => {
            NodeRecord::new(NodeKind::Whatsit, 9, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfRunningLink(running) => {
            NodeRecord::new(NodeKind::Whatsit, 10, bool_word(running), [0; 7])
        }
        Whatsit::PdfLiteral { mode, payload } => {
            let key = append_bytes::<ByteSpan>(annex, &payload);
            NodeRecord::with_key(NodeKind::Whatsit, 11, encode_literal_mode(mode), key)
        }
        Whatsit::DeferredPdfLiteral { mode, tokens } => {
            let key = tokens.coordinates();
            NodeRecord::new(
                NodeKind::Whatsit,
                12,
                encode_literal_mode(mode),
                [key[0], key[1], key[2], key[3], key[4], key[5], 0],
            )
        }
        Whatsit::PdfSetMatrix { payload } => NodeRecord::with_key(
            NodeKind::Whatsit,
            13,
            0,
            append_bytes::<ByteSpan>(annex, &payload),
        ),
        Whatsit::PdfSave => NodeRecord::new(NodeKind::Whatsit, 14, 0, [0; 7]),
        Whatsit::PdfRestore => NodeRecord::new(NodeKind::Whatsit, 15, 0, [0; 7]),
        Whatsit::PdfColorStack { id, action } => {
            let (action, key) = match action {
                crate::PdfColorStackAction::Set(bytes) => {
                    (0, Some(append_bytes::<ByteSpan>(annex, &bytes)))
                }
                crate::PdfColorStackAction::Push(bytes) => {
                    (1, Some(append_bytes::<ByteSpan>(annex, &bytes)))
                }
                crate::PdfColorStackAction::Pop => (2, None),
                crate::PdfColorStackAction::Current => (3, None),
            };
            let mut body = Vec::with_capacity(10);
            body.extend([id, action, bool_word(key.is_some())]);
            body.extend(key.map_or([0; 7], AnnexKey::words));
            NodeRecord::with_key(
                NodeKind::Whatsit,
                16,
                0,
                annex.append_fixed::<PdfColorStackPayload>(&body),
            )
        }
        Whatsit::PdfSavePos => NodeRecord::new(NodeKind::Whatsit, 17, 0, [0; 7]),
        Whatsit::PdfSnapRefPoint => NodeRecord::new(NodeKind::Whatsit, 18, 0, [0; 7]),
        Whatsit::PdfSnapY { glue } => {
            let glue = encode_glue(glue);
            NodeRecord::new(
                NodeKind::Whatsit,
                19,
                0,
                [glue[0], glue[1], glue[2], glue[3], 0, 0, 0],
            )
        }
        Whatsit::PdfSnapYComp { ratio } => NodeRecord::new(
            NodeKind::Whatsit,
            20,
            0,
            [u32::from(ratio), 0, 0, 0, 0, 0, 0],
        ),
        Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        }
        | Whatsit::PdfRefXImage {
            object,
            width,
            height,
            depth,
        } => NodeRecord::new(
            NodeKind::Whatsit,
            if matches!(value, Whatsit::PdfRefXImage { .. }) {
                22
            } else {
                21
            },
            0,
            [
                object,
                scaled_word(width),
                scaled_word(height),
                scaled_word(depth),
                0,
                0,
                0,
            ],
        ),
        Whatsit::PdfDestination(destination) => {
            let (identifier_tag, identifier) = encode_identifier(destination.identifier);
            let (kind_tag, kind_words, kind_presence) = encode_destination_kind(destination.kind);
            let mut body = Vec::with_capacity(12);
            body.push(identifier_tag);
            append_words(&mut body, identifier);
            body.push(destination.structure.unwrap_or_default());
            body.push(kind_tag);
            append_words(&mut body, kind_words);
            NodeRecord::with_key(
                NodeKind::Whatsit,
                23,
                bool_word(destination.structure.is_some()) | (kind_presence << 1),
                annex.append_fixed::<PdfDestinationPayload>(&body),
            )
        }
        Whatsit::PdfThread(thread) => {
            let (identifier_tag, identifier) = encode_identifier(thread.identifier);
            let (dimensions, presence) = encode_pdf_dimensions(thread.dimensions);
            let mut body = Vec::with_capacity(17);
            body.push(identifier_tag);
            append_words(&mut body, identifier);
            append_words(&mut body, dimensions);
            append_words(&mut body, thread.attributes.coordinates());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                24,
                presence | (bool_word(thread.running) << 3),
                annex.append_fixed::<PdfThreadPayload>(&body),
            )
        }
        Whatsit::PdfEndThread => NodeRecord::new(NodeKind::Whatsit, 25, 0, [0; 7]),
        Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        } => NodeRecord::new(
            NodeKind::Whatsit,
            26,
            0,
            [
                u32::from(language)
                    | (u32::from(left_hyphen_min) << 8)
                    | (u32::from(right_hyphen_min) << 16),
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        ),
    }
}

fn encode_literal_mode(value: PdfLiteralMode) -> u32 {
    match value {
        PdfLiteralMode::Origin => 0,
        PdfLiteralMode::Page => 1,
        PdfLiteralMode::Direct => 2,
    }
}

fn decode_literal_mode(value: u32) -> Option<PdfLiteralMode> {
    match value {
        0 => Some(PdfLiteralMode::Origin),
        1 => Some(PdfLiteralMode::Page),
        2 => Some(PdfLiteralMode::Direct),
        _ => None,
    }
}

impl NodeRecord<PageMaterialLane> {
    pub(super) fn reencode_whatsit(
        self,
        annex: &mut NodeAnnexCopier<'_>,
    ) -> Option<(Self, Option<usize>)> {
        let subtype = self.subtype();
        let flags = self.flags();
        match subtype {
            0 => {
                let mut payload =
                    annex.resolve_fixed_array::<OpenOutPayload, 8>(key_from_record(self))?;
                let path_key = AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?);
                let path = annex.detach_span(path_key)?;
                let path_key = annex.append_span::<Utf8Span>(&path);
                payload[..7].copy_from_slice(&path_key.words());
                let key = annex.append_fixed::<OpenOutPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            3 => {
                let mut payload =
                    annex.resolve_fixed_array::<SpecialPayload, 14>(key_from_record(self))?;
                let class_key = AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?);
                let bytes_key = AnnexKey::<ByteSpan>::from_words(payload[7..].try_into().ok()?);
                let class = annex.detach_span(class_key)?;
                let bytes = annex.detach_span(bytes_key)?;
                let class_key = annex.append_span::<Utf8Span>(&class);
                let bytes_key = annex.append_span::<ByteSpan>(&bytes);
                payload[..7].copy_from_slice(&class_key.words());
                payload[7..].copy_from_slice(&bytes_key.words());
                let key = annex.append_fixed::<SpecialPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            4 => {
                let mut payload = annex
                    .resolve_fixed_array::<DeferredSpecialPayload, 13>(key_from_record(self))?;
                let class_key = AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?);
                let class = annex.detach_span(class_key)?;
                let class_key = annex.append_span::<Utf8Span>(&class);
                payload[..7].copy_from_slice(&class_key.words());
                let key = annex.append_fixed::<DeferredSpecialPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            11 | 13 => {
                let bytes = annex.detach_span(key_from_record::<ByteSpan>(self))?;
                let key = annex.append_span::<ByteSpan>(&bytes);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            16 => {
                let mut payload =
                    annex.resolve_fixed_array::<PdfColorStackPayload, 10>(key_from_record(self))?;
                let bytes = match payload[2] {
                    0 => None,
                    1 => Some(annex.detach_span(AnnexKey::<ByteSpan>::from_words(
                        payload[3..].try_into().ok()?,
                    ))?),
                    _ => return None,
                };
                if let Some(bytes) = bytes {
                    let key = annex.append_span::<ByteSpan>(&bytes);
                    payload[3..].copy_from_slice(&key.words());
                }
                let key = annex.append_fixed::<PdfColorStackPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            23 => {
                let payload = annex
                    .resolve_fixed_array::<PdfDestinationPayload, 12>(key_from_record(self))?;
                let key = annex.append_fixed::<PdfDestinationPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            24 => {
                let payload =
                    annex.resolve_fixed_array::<PdfThreadPayload, 16>(key_from_record(self))?;
                let key = annex.append_fixed::<PdfThreadPayload>(&payload);
                Some((
                    Self::with_key(NodeKind::Whatsit, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            _ => Some((self, None)),
        }
    }
}

pub(super) fn decode_whatsit(record: NodeRecord, annex: NodeAnnexView<'_>) -> Option<Node> {
    decode_whatsit_value(record, annex).map(Node::Whatsit)
}

pub(super) fn decode_whatsit_value(
    record: NodeRecord,
    annex: NodeAnnexView<'_>,
) -> Option<Whatsit> {
    let subtype = record.subtype();
    let flags = record.flags();
    let words = record.words();
    let zero_tail = |start: usize| words[start..].iter().all(|word| *word == 0);
    let value = match subtype {
        0 if flags == 0 => {
            let payload = annex.resolve_fixed_shared(key_from_record::<OpenOutPayload>(record))?;
            if payload.len() != 8 || payload[7] >= 16 {
                return None;
            }
            let path = detach_bytes(
                annex,
                AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?),
            )?;
            Whatsit::OpenOut {
                slot: StreamSlot::new(payload[7] as u8),
                path: String::from_utf8(path).ok()?,
            }
        }
        1 if flags <= 1 && zero_tail(1) && words[0] < 16 => Whatsit::CloseOut {
            slot: (flags == 1).then(|| StreamSlot::new(words[0] as u8)),
        },
        2 if flags == 0 => Whatsit::DeferredWrite {
            sink: decode_print_sink(words[6])?,
            tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
        },
        3 if flags == 0 => {
            let payload = annex.resolve_fixed_shared(key_from_record::<SpecialPayload>(record))?;
            if payload.len() != 14 {
                return None;
            }
            Whatsit::Special {
                class: String::from_utf8(detach_bytes(
                    annex,
                    AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?),
                )?)
                .ok()?,
                payload: detach_bytes(
                    annex,
                    AnnexKey::<ByteSpan>::from_words(payload[7..].try_into().ok()?),
                )?,
            }
        }
        4 if flags == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<DeferredSpecialPayload>(record))?;
            if payload.len() != 13 {
                return None;
            }
            Whatsit::DeferredSpecial {
                class: String::from_utf8(detach_bytes(
                    annex,
                    AnnexKey::<Utf8Span>::from_words(payload[..7].try_into().ok()?),
                )?)
                .ok()?,
                tokens: NodeTokenKey::from_coordinates(payload[7..].try_into().ok()?),
            }
        }
        5 if flags == 0 && zero_tail(1) => Whatsit::PdfReferenceObject { object: words[0] },
        6 if flags <= 2 && words.iter().all(|word| *word == 0) => {
            Whatsit::PdfAccessibility(match flags {
                0 => PdfAccessibilityControl::InterwordSpaceOn,
                1 => PdfAccessibilityControl::InterwordSpaceOff,
                2 => PdfAccessibilityControl::FakeSpace,
                _ => return None,
            })
        }
        7 if flags == 0 && zero_tail(1) => Whatsit::PdfAnnotation { object: words[0] },
        8 if flags == 0 && zero_tail(1) => Whatsit::PdfLinkStart { object: words[0] },
        9 if flags == 0 && zero_tail(1) => Whatsit::PdfLinkEnd { object: words[0] },
        10 if flags <= 1 && words.iter().all(|word| *word == 0) => {
            Whatsit::PdfRunningLink(flags == 1)
        }
        11 if flags <= 2 => Whatsit::PdfLiteral {
            mode: decode_literal_mode(flags)?,
            payload: detach_bytes(annex, key_from_record::<ByteSpan>(record))?,
        },
        12 if flags <= 2 && words[6] == 0 => Whatsit::DeferredPdfLiteral {
            mode: decode_literal_mode(flags)?,
            tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
        },
        13 if flags == 0 => Whatsit::PdfSetMatrix {
            payload: detach_bytes(annex, key_from_record::<ByteSpan>(record))?,
        },
        14 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSave,
        15 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfRestore,
        16 if flags == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<PdfColorStackPayload>(record))?;
            if payload.len() != 10 {
                return None;
            }
            let present = decode_bool(payload[2])?;
            let key = AnnexKey::<ByteSpan>::from_words(payload[3..].try_into().ok()?);
            let bytes = || detach_bytes(annex, key);
            let action = match (payload[1], present) {
                (0, true) => crate::PdfColorStackAction::Set(bytes()?),
                (1, true) => crate::PdfColorStackAction::Push(bytes()?),
                (2, false) => crate::PdfColorStackAction::Pop,
                (3, false) => crate::PdfColorStackAction::Current,
                _ => return None,
            };
            Whatsit::PdfColorStack {
                id: payload[0],
                action,
            }
        }
        17 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSavePos,
        18 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSnapRefPoint,
        19 if flags == 0 && zero_tail(4) => Whatsit::PdfSnapY {
            glue: decode_glue(words[..4].try_into().ok()?)?,
        },
        20 if flags == 0 && words[0] <= u16::MAX as u32 && zero_tail(1) => Whatsit::PdfSnapYComp {
            ratio: words[0] as u16,
        },
        kind @ (21 | 22) if flags == 0 && zero_tail(4) => {
            let fields = (
                words[0],
                decode_scaled(words[1]),
                decode_scaled(words[2]),
                decode_scaled(words[3]),
            );
            if kind == 21 {
                Whatsit::PdfRefXForm {
                    object: fields.0,
                    width: fields.1,
                    height: fields.2,
                    depth: fields.3,
                }
            } else {
                Whatsit::PdfRefXImage {
                    object: fields.0,
                    width: fields.1,
                    height: fields.2,
                    depth: fields.3,
                }
            }
        }
        23 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<PdfDestinationPayload>(record))?;
            if payload.len() != 12 || flags >> 8 != 0 {
                return None;
            }
            let identifier = decode_identifier(payload[0], payload[1..7].try_into().ok()?)?;
            let structure = (flags & 1 != 0).then_some(payload[7]);
            let kind =
                decode_destination_kind(payload[8], payload[9..12].try_into().ok()?, flags >> 1)?;
            Whatsit::PdfDestination(Box::new(PdfDestinationNode {
                identifier,
                structure,
                kind,
            }))
        }
        24 if flags & !0xf == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<PdfThreadPayload>(record))?;
            if payload.len() != 16 {
                return None;
            }
            Whatsit::PdfThread(Box::new(PdfThreadNode {
                identifier: decode_identifier(payload[0], payload[1..7].try_into().ok()?)?,
                dimensions: decode_pdf_dimensions(payload[7..10].try_into().ok()?, flags & 7)?,
                attributes: NodeTokenKey::from_coordinates(payload[10..16].try_into().ok()?),
                running: flags & 8 != 0,
            }))
        }
        25 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfEndThread,
        26 if flags == 0 && zero_tail(1) && words[0] >> 24 == 0 => Whatsit::Language {
            language: words[0] as u8,
            left_hyphen_min: (words[0] >> 8) as u8,
            right_hyphen_min: (words[0] >> 16) as u8,
        },
        _ => return None,
    };
    Some(value)
}
