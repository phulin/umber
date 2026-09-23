use super::*;

impl Reader<'_> {
    pub(super) fn fonts(&mut self) -> Result<Vec<FontResource>, ParseError> {
        let len = self.collection_len(127)?;
        let mut fonts = Vec::with_capacity(len);
        for _ in 0..len {
            let font_id = self.u32()?;
            let name = self.str()?;
            let tfm_content_hash = self.ahash64_identity()?;
            let tfm_checksum = self.u32()?;
            let design_size = self.scaled()?;
            let at_size = self.scaled()?;
            let (layout_policy, mapping_fallback) = {
                let policy = match self.u8()? {
                    1 => tex_fonts::FontLayoutPolicy::OpenTypePreferred,
                    2 => tex_fonts::FontLayoutPolicy::ClassicTfmExact,
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "font layout policy",
                            tag,
                        });
                    }
                };
                let fallback = match self.u8()? {
                    0 => None,
                    1 => Some(tex_fonts::FontMappingFallbackPolicy::Error),
                    2 => Some(tex_fonts::FontMappingFallbackPolicy::ClassicTfmExact),
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "font mapping fallback",
                            tag,
                        });
                    }
                };
                (policy, fallback)
            };
            let opentype = {
                match self.u8()? {
                    0 => None,
                    1 => {
                        let program_identity =
                            tex_fonts::FontProgramIdentity::from_bytes(self.ahash64_identity()?);
                        let object_identity =
                            tex_fonts::FontObjectIdentity::from_bytes(self.ahash64_identity()?);
                        let instance_identity =
                            tex_fonts::FontInstanceIdentity::from_bytes(self.ahash64_identity()?);
                        let container = match self.u8()? {
                            1 => tex_fonts::FontContainer::OpenType,
                            2 => tex_fonts::FontContainer::TrueType,
                            3 => tex_fonts::FontContainer::Collection,
                            4 => tex_fonts::FontContainer::Woff2,
                            tag => {
                                return Err(ParseError::InvalidTag {
                                    kind: "font container",
                                    tag,
                                });
                            }
                        };
                        let (face_index, variation, features, direction, script, language) = {
                            let face_index = self.u32()?;
                            let variation_kind = self.u8()?;
                            let named_id = if variation_kind == 1 {
                                Some(self.u16()?)
                            } else {
                                None
                            };
                            if variation_kind > 2 {
                                return Err(ParseError::InvalidTag {
                                    kind: "variation instance",
                                    tag: variation_kind,
                                });
                            }
                            let coordinate_count = self.collection_len(8)?;
                            let mut coordinates = Vec::with_capacity(coordinate_count);
                            for _ in 0..coordinate_count {
                                coordinates.push(tex_fonts::VariationCoordinate {
                                    tag: tex_fonts::OpenTypeTag::new(self.opentype_tag()?),
                                    value: self.i32()?,
                                });
                            }
                            let variation = match (variation_kind, named_id) {
                                (0, _) if coordinates.is_empty() => {
                                    Ok(tex_fonts::VariationSelection::default())
                                }
                                (1, Some(name_id)) => {
                                    tex_fonts::VariationSelection::resolved_named(
                                        name_id,
                                        coordinates,
                                    )
                                }
                                (2, _) => tex_fonts::VariationSelection::new(coordinates),
                                _ => Err(tex_fonts::FontSelectionError::DuplicateVariationAxis),
                            }
                            .map_err(|_| ParseError::InvalidTag {
                                kind: "variation selection",
                                tag: variation_kind,
                            })?;
                            let feature_count = self.collection_len(8)?;
                            let mut feature_settings = Vec::with_capacity(feature_count);
                            for _ in 0..feature_count {
                                feature_settings.push(tex_fonts::FeatureSetting {
                                    tag: tex_fonts::OpenTypeTag::new(self.opentype_tag()?),
                                    value: self.u32()?,
                                });
                            }
                            let features = tex_fonts::FontFeaturePolicy::new(feature_settings)
                                .map_err(|_| ParseError::InvalidTag {
                                    kind: "feature policy",
                                    tag: 0,
                                })?;
                            let direction = match self.u8()? {
                                1 => tex_fonts::WritingDirection::LeftToRight,
                                2 => tex_fonts::WritingDirection::RightToLeft,
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "writing direction",
                                        tag,
                                    });
                                }
                            };
                            let script = match self.u8()? {
                                0 => None,
                                1 => Some(tex_fonts::OpenTypeTag::new(self.opentype_tag()?)),
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "script",
                                        tag,
                                    });
                                }
                            };
                            let language = match self.u8()? {
                                0 => None,
                                1 => Some(tex_fonts::FontLanguage::new(self.str()?).map_err(
                                    |_| ParseError::InvalidTag {
                                        kind: "language",
                                        tag: 0,
                                    },
                                )?),
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "language",
                                        tag,
                                    });
                                }
                            };
                            (face_index, variation, features, direction, script, language)
                        };
                        let (
                            encoding_map_version,
                            encoding_map_identity,
                            fontdimen_synthesis_version,
                        ) = {
                            let map_version = self.optional_u8("encoding map version")?;
                            let map_identity = match self.u8()? {
                                0 => None,
                                1 => Some(self.ahash64_identity()?),
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "encoding map identity",
                                        tag,
                                    });
                                }
                            };
                            let fontdimen = self.optional_u8("fontdimen synthesis version")?;
                            (map_version, map_identity, fontdimen)
                        };
                        Some(crate::OpenTypeFontResource {
                            program_identity,
                            object_identity,
                            instance_identity,
                            container,
                            face_index,
                            variation,
                            features,
                            direction,
                            script,
                            language,
                            encoding_map_version,
                            encoding_map_identity,
                            fontdimen_synthesis_version,
                        })
                    }
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "optional OpenType font",
                            tag,
                        });
                    }
                }
            };
            let (semantic_identity, construction) = {
                let semantic_identity =
                    tex_fonts::FontSourceIdentity::from_bytes(self.ahash64_identity()?);
                let tag = self.u8()?;
                let construction = match tag {
                    wire::font_construction::LOADED => FontResourceConstruction::Loaded,
                    wire::font_construction::COPIED => FontResourceConstruction::Copied {
                        source_font_id: self.u32()?,
                        source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                            self.ahash64_identity()?,
                        ),
                    },
                    wire::font_construction::LETTERSPACED => {
                        FontResourceConstruction::Letterspaced {
                            source_font_id: self.u32()?,
                            source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                                self.ahash64_identity()?,
                            ),
                            amount: self.u16()? as i16,
                            no_ligatures: match self.u8()? {
                                0 => false,
                                1 => true,
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "letterspace no-ligatures flag",
                                        tag,
                                    });
                                }
                            },
                        }
                    }
                    wire::font_construction::EXPANDED => FontResourceConstruction::Expanded {
                        source_font_id: self.u32()?,
                        source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                            self.ahash64_identity()?,
                        ),
                        ratio: self.u16()? as i16,
                    },
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "font construction",
                            tag,
                        });
                    }
                };
                (semantic_identity, construction)
            };
            fonts.push(FontResource {
                font_id,
                name,
                tfm_content_hash,
                tfm_checksum,
                design_size,
                at_size,
                layout_policy,
                mapping_fallback,
                opentype,
                semantic_identity,
                construction,
            });
        }
        Ok(fonts)
    }

    pub(super) fn ahash64_identity(&mut self) -> Result<[u8; 8], ParseError> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(bytes)
    }

    pub(super) fn opentype_tag(&mut self) -> Result<[u8; 4], ParseError> {
        self.take(4)?
            .try_into()
            .map_err(|_| ParseError::UnexpectedEof)
    }

    pub(super) fn effects(&mut self) -> Result<Vec<PageEffect>, ParseError> {
        let len = self.collection_len(1)?;
        let mut effects = Vec::with_capacity(len);
        for _ in 0..len {
            let tag = self.u8()?;
            effects.push(match tag {
                wire::effect::OPEN_OUT => PageEffect::OpenOut {
                    stream: self.u8()?,
                    path: self.str()?,
                },
                wire::effect::CLOSE_OUT => PageEffect::CloseOut { stream: self.u8()? },
                wire::effect::WRITE => PageEffect::Write {
                    sink: self.sink()?,
                    text: self.str()?,
                },
                wire::effect::SPECIAL => PageEffect::Special {
                    class: self.str()?,
                    payload: self.bytes()?,
                },
                wire::effect::PDF_ACCESSIBILITY => {
                    PageEffect::PdfAccessibility(match self.u8()? {
                        0 => PdfAccessibilityEffect::InterwordSpaceOn,
                        1 => PdfAccessibilityEffect::InterwordSpaceOff,
                        2 => PdfAccessibilityEffect::FakeSpace,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF accessibility effect",
                                tag,
                            });
                        }
                    })
                }
                wire::effect::PDF_ANNOTATION => PageEffect::PdfAnnotation(match self.u8()? {
                    0 => PdfAnnotationEffect::Annotation {
                        object: self.u32()?,
                    },
                    1 => PdfAnnotationEffect::LinkStart {
                        object: self.u32()?,
                    },
                    2 => PdfAnnotationEffect::LinkEnd {
                        object: self.u32()?,
                    },
                    3 => PdfAnnotationEffect::RunningLink(match self.u8()? {
                        0 => false,
                        1 => true,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF running-link boolean",
                                tag,
                            });
                        }
                    }),
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "PDF annotation effect",
                            tag,
                        });
                    }
                }),
                wire::effect::PDF_LITERAL => PageEffect::PdfLiteral {
                    mode: match self.u8()? {
                        0 => PdfLiteralMode::Origin,
                        1 => PdfLiteralMode::Page,
                        2 => PdfLiteralMode::Direct,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF literal mode",
                                tag,
                            });
                        }
                    },
                    payload: self.bytes()?,
                },
                wire::effect::PDF_SET_MATRIX => PageEffect::PdfSetMatrix {
                    payload: self.bytes()?,
                },
                wire::effect::PDF_SAVE => PageEffect::PdfSave,
                wire::effect::PDF_RESTORE => PageEffect::PdfRestore,
                wire::effect::PDF_COLOR_STACK => PageEffect::PdfColorStack {
                    mode: match self.u8()? {
                        0 => PdfLiteralMode::Origin,
                        1 => PdfLiteralMode::Page,
                        2 => PdfLiteralMode::Direct,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF color stack mode",
                                tag,
                            });
                        }
                    },
                    page_start: match self.u8()? {
                        0 => false,
                        1 => true,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "boolean",
                                tag,
                            });
                        }
                    },
                    payload: self.bytes()?,
                },
                wire::effect::PDF_SAVE_POSITION => PageEffect::PdfSavePosition,
                wire::effect::PDF_SNAP_STATE => PageEffect::PdfSnapState {
                    x: self.scaled()?,
                    y: self.scaled()?,
                },
                wire::effect::PDF_SNAP_REF_POINT => PageEffect::PdfSnapRefPoint,
                wire::effect::PDF_SNAP_Y => PageEffect::PdfSnapY {
                    spec: self.glue_spec()?,
                },
                wire::effect::PDF_SNAP_Y_COMP => PageEffect::PdfSnapYComp { ratio: self.u16()? },
                wire::effect::PDF_REF_XFORM => PageEffect::PdfRefXForm {
                    object: self.u32()?,
                    width: self.scaled()?,
                    height: self.scaled()?,
                    depth: self.scaled()?,
                },
                wire::effect::PDF_REF_XIMAGE => PageEffect::PdfRefXImage {
                    object: self.u32()?,
                    width: self.scaled()?,
                    height: self.scaled()?,
                    depth: self.scaled()?,
                },
                wire::effect::PDF_DESTINATION => {
                    let object = self.u32()?;
                    let identifier = match self.u8()? {
                        0 => PdfDestinationIdentifier::Name(self.bytes()?),
                        1 => PdfDestinationIdentifier::Number(self.u32()?),
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF destination identifier",
                                tag,
                            });
                        }
                    };
                    let structure = match self.u8()? {
                        0 => None,
                        1 => Some(self.u32()?),
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF destination structure",
                                tag,
                            });
                        }
                    };
                    let kind = match self.u8()? {
                        0 => PdfDestinationKind::Xyz {
                            zoom: match self.u8()? {
                                0 => None,
                                1 => Some(self.i32()?),
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "PDF destination zoom",
                                        tag,
                                    });
                                }
                            },
                        },
                        1 => PdfDestinationKind::FitBoundingBoxHorizontal,
                        2 => PdfDestinationKind::FitBoundingBoxVertical,
                        3 => PdfDestinationKind::FitBoundingBox,
                        4 => PdfDestinationKind::FitHorizontal,
                        5 => PdfDestinationKind::FitVertical,
                        6 => {
                            let mut read = || -> Result<Option<Scaled>, ParseError> {
                                Ok(match self.u8()? {
                                    0 => None,
                                    1 => Some(self.scaled()?),
                                    tag => {
                                        return Err(ParseError::InvalidTag {
                                            kind: "PDF destination dimension",
                                            tag,
                                        });
                                    }
                                })
                            };
                            PdfDestinationKind::FitRectangle {
                                width: read()?,
                                height: read()?,
                                depth: read()?,
                            }
                        }
                        7 => PdfDestinationKind::Fit,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF destination kind",
                                tag,
                            });
                        }
                    };
                    PageEffect::PdfDestination(PdfDestinationEffect {
                        object,
                        identifier,
                        structure,
                        kind,
                        margin: self.scaled()?,
                    })
                }
                tag @ (wire::effect::PDF_THREAD | wire::effect::PDF_START_THREAD) => {
                    let thread_object = self.u32()?;
                    let bead_object = self.u32()?;
                    let rectangle_object = self.u32()?;
                    let identifier = match self.u8()? {
                        0 => PdfDestinationIdentifier::Name(self.bytes()?),
                        1 => PdfDestinationIdentifier::Number(self.u32()?),
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF thread identifier",
                                tag,
                            });
                        }
                    };
                    let mut dimension = || -> Result<Option<Scaled>, ParseError> {
                        match self.u8()? {
                            0 => Ok(None),
                            1 => Ok(Some(self.scaled()?)),
                            tag => Err(ParseError::InvalidTag {
                                kind: "PDF thread dimension",
                                tag,
                            }),
                        }
                    };
                    let marker = PdfThreadEffect {
                        thread_object,
                        bead_object,
                        rectangle_object,
                        identifier,
                        width: dimension()?,
                        height: dimension()?,
                        depth: dimension()?,
                        attributes: self.bytes()?,
                        margin: self.scaled()?,
                    };
                    if tag == wire::effect::PDF_THREAD {
                        PageEffect::PdfThread(marker)
                    } else {
                        PageEffect::PdfStartThread(marker)
                    }
                }
                wire::effect::PDF_END_THREAD => PageEffect::PdfEndThread,
                tag => {
                    return Err(ParseError::InvalidTag {
                        kind: "effect",
                        tag,
                    });
                }
            });
        }
        Ok(effects)
    }

    pub(super) fn math_events(&mut self) -> Result<Vec<MathOutputEvent>, ParseError> {
        let len = self.collection_len(1)?;
        let mut events = Vec::with_capacity(len);
        for _ in 0..len {
            events.push(match self.u8()? {
                wire::math_event::START => MathOutputEvent::Start(crate::MathStart {
                    id: self.u32()?,
                    x: self.scaled()?,
                    baseline: self.scaled()?,
                    width: self.scaled()?,
                    height: self.scaled()?,
                    depth: self.scaled()?,
                }),
                wire::math_event::GLYPH => {
                    let font_instance =
                        tex_fonts::FontInstanceIdentity::from_bytes(self.ahash64_identity()?);
                    let glyph_id = self.u16()?;
                    let selection = match self.u8()? {
                        wire::math_selection::CMAP => MathGlyphSelection::Cmap {
                            scalar: self.u32()?,
                        },
                        wire::math_selection::OUTLINE_FALLBACK => {
                            MathGlyphSelection::OutlineFallback
                        }
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "math glyph selection",
                                tag,
                            });
                        }
                    };
                    MathOutputEvent::Glyph(crate::MathGlyph {
                        font_instance,
                        glyph_id,
                        selection,
                        ssty: self.u8()?,
                        x: self.scaled()?,
                        baseline: self.scaled()?,
                        width: self.scaled()?,
                        height: self.scaled()?,
                        depth: self.scaled()?,
                    })
                }
                wire::math_event::RULE => MathOutputEvent::Rule(crate::MathRule {
                    x: self.scaled()?,
                    y: self.scaled()?,
                    width: self.scaled()?,
                    height: self.scaled()?,
                }),
                wire::math_event::END => MathOutputEvent::End,
                tag => {
                    return Err(ParseError::InvalidTag {
                        kind: "math event",
                        tag,
                    });
                }
            });
        }
        Ok(events)
    }

    pub(super) fn sink(&mut self) -> Result<EffectSink, ParseError> {
        match self.u8()? {
            wire::sink::TERMINAL => Ok(EffectSink::Terminal),
            wire::sink::LOG => Ok(EffectSink::Log),
            wire::sink::TERMINAL_AND_LOG => Ok(EffectSink::TerminalAndLog),
            wire::sink::STREAM => Ok(EffectSink::Stream(self.u8()?)),
            tag => Err(ParseError::InvalidTag { kind: "sink", tag }),
        }
    }
}
