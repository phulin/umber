use super::*;

impl Writer {
    pub(super) fn fonts(&mut self, fonts: &[FontResource]) {
        self.collection_len(fonts.len());
        for font in fonts {
            if self.error.is_some() {
                return;
            }
            self.u32(font.font_id);
            self.str(&font.name);
            self.raw(&font.tfm_content_hash);
            self.u32(font.tfm_checksum);
            self.scaled(font.design_size);
            self.scaled(font.at_size);
            self.u8(match font.layout_policy {
                tex_fonts::FontLayoutPolicy::OpenTypePreferred => 1,
                tex_fonts::FontLayoutPolicy::ClassicTfmExact => 2,
            });
            self.u8(match font.mapping_fallback {
                None => 0,
                Some(tex_fonts::FontMappingFallbackPolicy::Error) => 1,
                Some(tex_fonts::FontMappingFallbackPolicy::ClassicTfmExact) => 2,
            });
            match &font.opentype {
                Some(opentype) => {
                    self.u8(1);
                    self.raw(&opentype.program_identity.bytes());
                    self.raw(&opentype.object_identity.bytes());
                    self.raw(&opentype.instance_identity.bytes());
                    self.u8(opentype.container as u8);
                    self.u32(opentype.face_index);
                    match opentype.variation.instance() {
                        tex_fonts::VariationInstance::Default => self.u8(0),
                        tex_fonts::VariationInstance::Named(name_id) => {
                            self.u8(1);
                            self.u16(name_id);
                        }
                        tex_fonts::VariationInstance::Coordinates => self.u8(2),
                    }
                    self.collection_len(opentype.variation.coordinates().len());
                    for coordinate in opentype.variation.coordinates() {
                        self.raw(&coordinate.tag.bytes());
                        self.i32(coordinate.value);
                    }
                    self.collection_len(opentype.features.settings().len());
                    for feature in opentype.features.settings() {
                        self.raw(&feature.tag.bytes());
                        self.u32(feature.value);
                    }
                    self.u8(opentype.direction as u8);
                    match opentype.script {
                        Some(script) => {
                            self.u8(1);
                            self.raw(&script.bytes());
                        }
                        None => self.u8(0),
                    }
                    match &opentype.language {
                        Some(language) => {
                            self.u8(1);
                            self.str(language.as_str());
                        }
                        None => self.u8(0),
                    }
                    self.optional_u8(opentype.encoding_map_version);
                    match opentype.encoding_map_identity {
                        Some(identity) => {
                            self.u8(1);
                            self.raw(&identity);
                        }
                        None => self.u8(0),
                    }
                    self.optional_u8(opentype.fontdimen_synthesis_version);
                }
                None => self.u8(0),
            }
            self.raw(&font.semantic_identity.bytes());
            match font.construction {
                FontResourceConstruction::Loaded => self.u8(wire::font_construction::LOADED),
                FontResourceConstruction::Copied {
                    source_font_id,
                    source_identity,
                } => {
                    self.u8(wire::font_construction::COPIED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                }
                FontResourceConstruction::Letterspaced {
                    source_font_id,
                    source_identity,
                    amount,
                    no_ligatures,
                } => {
                    self.u8(wire::font_construction::LETTERSPACED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                    self.raw(&amount.to_le_bytes());
                    self.u8(u8::from(no_ligatures));
                }
                FontResourceConstruction::Expanded {
                    source_font_id,
                    source_identity,
                    ratio,
                } => {
                    self.u8(wire::font_construction::EXPANDED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                    self.raw(&ratio.to_le_bytes());
                }
            }
        }
    }

    pub(super) fn effects(&mut self, effects: &[PageEffect]) {
        self.collection_len(effects.len());
        for effect in effects {
            if self.error.is_some() {
                return;
            }
            match effect {
                PageEffect::OpenOut { stream, path } => {
                    self.u8(wire::effect::OPEN_OUT);
                    self.u8(*stream);
                    self.str(path);
                }
                PageEffect::CloseOut { stream } => {
                    self.u8(wire::effect::CLOSE_OUT);
                    self.u8(*stream);
                }
                PageEffect::Write { sink, text } => {
                    self.u8(wire::effect::WRITE);
                    self.sink(*sink);
                    self.str(text);
                }
                PageEffect::Special { class, payload } => {
                    self.u8(wire::effect::SPECIAL);
                    self.str(class);
                    self.bytes(payload);
                }
                PageEffect::PdfAccessibility(control) => {
                    self.u8(wire::effect::PDF_ACCESSIBILITY);
                    self.u8(match control {
                        PdfAccessibilityEffect::InterwordSpaceOn => 0,
                        PdfAccessibilityEffect::InterwordSpaceOff => 1,
                        PdfAccessibilityEffect::FakeSpace => 2,
                    });
                }
                PageEffect::PdfAnnotation(marker) => {
                    self.u8(wire::effect::PDF_ANNOTATION);
                    match marker {
                        PdfAnnotationEffect::Annotation { object } => {
                            self.u8(0);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::LinkStart { object } => {
                            self.u8(1);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::LinkEnd { object } => {
                            self.u8(2);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::RunningLink(enabled) => {
                            self.u8(3);
                            self.u8(u8::from(*enabled));
                        }
                    }
                }
                PageEffect::PdfLiteral { mode, payload } => {
                    self.u8(wire::effect::PDF_LITERAL);
                    self.u8(match mode {
                        PdfLiteralMode::Origin => 0,
                        PdfLiteralMode::Page => 1,
                        PdfLiteralMode::Direct => 2,
                    });
                    self.bytes(payload);
                }
                PageEffect::PdfSetMatrix { payload } => {
                    self.u8(wire::effect::PDF_SET_MATRIX);
                    self.bytes(payload);
                }
                PageEffect::PdfSave => self.u8(wire::effect::PDF_SAVE),
                PageEffect::PdfRestore => self.u8(wire::effect::PDF_RESTORE),
                PageEffect::PdfColorStack {
                    mode,
                    payload,
                    page_start,
                } => {
                    self.u8(wire::effect::PDF_COLOR_STACK);
                    self.u8(match mode {
                        PdfLiteralMode::Origin => 0,
                        PdfLiteralMode::Page => 1,
                        PdfLiteralMode::Direct => 2,
                    });
                    self.u8(u8::from(*page_start));
                    self.bytes(payload);
                }
                PageEffect::PdfSavePosition => self.u8(wire::effect::PDF_SAVE_POSITION),
                PageEffect::PdfSnapState { x, y } => {
                    self.u8(wire::effect::PDF_SNAP_STATE);
                    self.scaled(*x);
                    self.scaled(*y);
                }
                PageEffect::PdfSnapRefPoint => self.u8(wire::effect::PDF_SNAP_REF_POINT),
                PageEffect::PdfSnapY { spec } => {
                    self.u8(wire::effect::PDF_SNAP_Y);
                    self.glue_spec(*spec);
                }
                PageEffect::PdfSnapYComp { ratio } => {
                    self.u8(wire::effect::PDF_SNAP_Y_COMP);
                    self.u16(*ratio);
                }
                PageEffect::PdfRefXForm {
                    object,
                    width,
                    height,
                    depth,
                } => {
                    self.u8(wire::effect::PDF_REF_XFORM);
                    self.u32(*object);
                    self.scaled(*width);
                    self.scaled(*height);
                    self.scaled(*depth);
                }
                PageEffect::PdfRefXImage {
                    object,
                    width,
                    height,
                    depth,
                } => {
                    self.u8(wire::effect::PDF_REF_XIMAGE);
                    self.u32(*object);
                    self.scaled(*width);
                    self.scaled(*height);
                    self.scaled(*depth);
                }
                PageEffect::PdfDestination(marker) => {
                    self.u8(wire::effect::PDF_DESTINATION);
                    self.u32(marker.object);
                    match &marker.identifier {
                        PdfDestinationIdentifier::Name(name) => {
                            self.u8(0);
                            self.bytes(name);
                        }
                        PdfDestinationIdentifier::Number(number) => {
                            self.u8(1);
                            self.u32(*number);
                        }
                    }
                    self.u8(u8::from(marker.structure.is_some()));
                    if let Some(structure) = marker.structure {
                        self.u32(structure);
                    }
                    match marker.kind {
                        PdfDestinationKind::Xyz { zoom } => {
                            self.u8(0);
                            self.u8(u8::from(zoom.is_some()));
                            if let Some(zoom) = zoom {
                                self.i32(zoom);
                            }
                        }
                        PdfDestinationKind::FitBoundingBoxHorizontal => self.u8(1),
                        PdfDestinationKind::FitBoundingBoxVertical => self.u8(2),
                        PdfDestinationKind::FitBoundingBox => self.u8(3),
                        PdfDestinationKind::FitHorizontal => self.u8(4),
                        PdfDestinationKind::FitVertical => self.u8(5),
                        PdfDestinationKind::FitRectangle {
                            width,
                            height,
                            depth,
                        } => {
                            self.u8(6);
                            for value in [width, height, depth] {
                                self.u8(u8::from(value.is_some()));
                                if let Some(value) = value {
                                    self.scaled(value);
                                }
                            }
                        }
                        PdfDestinationKind::Fit => self.u8(7),
                    }
                    self.scaled(marker.margin);
                }
                PageEffect::PdfThread(marker) | PageEffect::PdfStartThread(marker) => {
                    self.u8(if matches!(effect, PageEffect::PdfThread(_)) {
                        wire::effect::PDF_THREAD
                    } else {
                        wire::effect::PDF_START_THREAD
                    });
                    self.u32(marker.thread_object);
                    self.u32(marker.bead_object);
                    self.u32(marker.rectangle_object);
                    match &marker.identifier {
                        PdfDestinationIdentifier::Name(name) => {
                            self.u8(0);
                            self.bytes(name);
                        }
                        PdfDestinationIdentifier::Number(number) => {
                            self.u8(1);
                            self.u32(*number);
                        }
                    }
                    for value in [marker.width, marker.height, marker.depth] {
                        self.u8(u8::from(value.is_some()));
                        if let Some(value) = value {
                            self.scaled(value);
                        }
                    }
                    self.bytes(&marker.attributes);
                    self.scaled(marker.margin);
                }
                PageEffect::PdfEndThread => self.u8(wire::effect::PDF_END_THREAD),
            }
        }
    }

    pub(super) fn math_events(&mut self, events: &[MathOutputEvent]) {
        self.collection_len(events.len());
        for event in events {
            if self.error.is_some() {
                return;
            }
            match event {
                MathOutputEvent::Start(start) => {
                    self.u8(wire::math_event::START);
                    self.u32(start.id);
                    self.scaled(start.x);
                    self.scaled(start.baseline);
                    self.scaled(start.width);
                    self.scaled(start.height);
                    self.scaled(start.depth);
                }
                MathOutputEvent::Glyph(glyph) => {
                    self.u8(wire::math_event::GLYPH);
                    self.raw(&glyph.font_instance.bytes());
                    self.u16(glyph.glyph_id);
                    match glyph.selection {
                        MathGlyphSelection::Cmap { scalar } => {
                            self.u8(wire::math_selection::CMAP);
                            self.u32(scalar);
                        }
                        MathGlyphSelection::OutlineFallback => {
                            self.u8(wire::math_selection::OUTLINE_FALLBACK);
                        }
                    }
                    self.u8(glyph.ssty);
                    self.scaled(glyph.x);
                    self.scaled(glyph.baseline);
                    self.scaled(glyph.width);
                    self.scaled(glyph.height);
                    self.scaled(glyph.depth);
                }
                MathOutputEvent::Rule(rule) => {
                    self.u8(wire::math_event::RULE);
                    self.scaled(rule.x);
                    self.scaled(rule.y);
                    self.scaled(rule.width);
                    self.scaled(rule.height);
                }
                MathOutputEvent::End => self.u8(wire::math_event::END),
            }
        }
    }

    pub(super) fn sink(&mut self, sink: EffectSink) {
        match sink {
            EffectSink::Terminal => self.u8(wire::sink::TERMINAL),
            EffectSink::Log => self.u8(wire::sink::LOG),
            EffectSink::TerminalAndLog => self.u8(wire::sink::TERMINAL_AND_LOG),
            EffectSink::Stream(stream) => {
                self.u8(wire::sink::STREAM);
                self.u8(stream);
            }
        }
    }

    pub(super) fn node(&mut self, node: &PageNode) {
        for event in crate::node_cursor::ArtifactNodeCursor::new(node) {
            if self.error.is_some() {
                return;
            }
            match event {
                crate::node_cursor::ArtifactNodeEvent::Node { node, depth } => {
                    self.write_node(node, depth);
                }
                crate::node_cursor::ArtifactNodeEvent::List { nodes } => {
                    self.collection_len(nodes.len());
                }
            }
        }
    }

    pub(super) fn write_node(&mut self, node: &PageNode, depth: usize) {
        if self.error.is_some() {
            return;
        }
        if depth > self.limits.max_depth {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Depth,
                actual: depth,
                limit: self.limits.max_depth,
            });
            return;
        }
        self.nodes_seen += 1;
        if self.nodes_seen > self.limits.max_nodes {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Nodes,
                actual: self.nodes_seen,
                limit: self.limits.max_nodes,
            });
            return;
        }
        self.node_head(node);
    }

    /// Emits only the scalar/header portion of a node. Nested collection
    /// lengths and children are supplied by `ArtifactNodeCursor` events.
    pub(super) fn node_head(&mut self, node: &PageNode) {
        match node {
            PageNode::Char { font_id, ch, width } => {
                let mut bytes = [0; 13];
                bytes[0] = wire::node::CHAR;
                bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
                bytes[5..9].copy_from_slice(&ch.to_le_bytes());
                bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
                self.raw(&bytes);
            }
            PageNode::Lig {
                font_id,
                ch,
                source,
                width,
            } => {
                let mut bytes = [0; 17];
                bytes[0] = wire::node::LIG;
                bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
                bytes[5..9].copy_from_slice(&ch.to_le_bytes());
                bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
                bytes[13..17].copy_from_slice(&(source.len() as u32).to_le_bytes());
                self.raw(&bytes);
                for code in source {
                    self.u32(*code);
                }
            }
            PageNode::Kern { amount, kind } => {
                let mut bytes = [0; 6];
                bytes[0] = wire::node::KERN;
                bytes[1..5].copy_from_slice(&amount.raw().to_le_bytes());
                bytes[5] = kern_kind_tag(*kind);
                self.raw(&bytes);
            }
            PageNode::MarginKern {
                amount,
                side,
                font_id,
                ch,
            } => {
                self.u8(wire::node::MARGIN_KERN);
                self.scaled(*amount);
                self.u8(u8::from(matches!(side, MarginKernSide::Right)));
                self.u32(*font_id);
                self.u8(*ch);
            }
            PageNode::Glue { spec, kind, leader } => {
                self.u8(wire::node::GLUE);
                self.glue_spec(*spec);
                self.u8(glue_kind_tag(*kind));
                match leader {
                    None => self.u8(wire::leader::NONE),
                    Some(LeaderPayload::HList(box_node)) => {
                        self.u8(wire::leader::HLIST);
                        self.box_fields(box_node);
                    }
                    Some(LeaderPayload::VList(box_node)) => {
                        self.u8(wire::leader::VLIST);
                        self.box_fields(box_node);
                    }
                    Some(LeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }) => {
                        self.u8(wire::leader::RULE);
                        self.optional_scaled(*width);
                        self.optional_scaled(*height);
                        self.optional_scaled(*depth);
                    }
                }
            }
            PageNode::Penalty(value) => {
                self.tagged_i32(wire::node::PENALTY, *value);
            }
            PageNode::Rule {
                width,
                height,
                depth,
            } => {
                self.u8(wire::node::RULE);
                self.optional_scaled(*width);
                self.optional_scaled(*height);
                self.optional_scaled(*depth);
            }
            PageNode::HList(box_node) => {
                self.u8(wire::node::HLIST);
                self.box_fields(box_node);
            }
            PageNode::VList(box_node) => {
                self.u8(wire::node::VLIST);
                self.box_fields(box_node);
            }
            PageNode::Disc { kind, .. } => {
                self.u8(wire::node::DISC);
                self.u8(disc_kind_tag(*kind));
            }
            PageNode::Mark { class, tokens } => {
                self.u8(wire::node::MARK);
                self.u16(*class);
                self.tokens(tokens);
            }
            PageNode::Insert { class, .. } => {
                self.u8(wire::node::INSERT);
                self.u16(*class);
            }
            PageNode::WhatsitAnchor { effect_index } => {
                self.tagged_u32(wire::node::WHATSIT_ANCHOR, *effect_index);
            }
            PageNode::MathOn(width) => {
                self.tagged_i32(wire::node::MATH_ON, width.raw());
            }
            PageNode::MathOff(width) => {
                self.tagged_i32(wire::node::MATH_OFF, width.raw());
            }
            PageNode::Adjust(_) => {
                self.u8(wire::node::ADJUST);
            }
        }
    }

    pub(super) fn tokens(&mut self, tokens: &[PageToken]) {
        self.collection_len(tokens.len());
        for token in tokens {
            if self.error.is_some() {
                return;
            }
            match token {
                PageToken::Char { ch, cat } => {
                    self.u8(wire::token::CHAR);
                    self.u32(*ch);
                    self.u8(token_catcode_tag(*cat));
                }
                PageToken::ControlSequence(name) => {
                    self.u8(wire::token::CONTROL_SEQUENCE);
                    self.str(name);
                }
                PageToken::Param(slot) => {
                    self.u8(wire::token::PARAM);
                    self.u8(*slot);
                }
                PageToken::ActiveControlSequence(ch) => {
                    self.u8(wire::token::ACTIVE_CONTROL_SEQUENCE);
                    self.u32(*ch);
                }
            }
        }
    }

    pub(super) fn tagged_i32(&mut self, tag: u8, value: i32) {
        let mut bytes = [0; 5];
        bytes[0] = tag;
        bytes[1..].copy_from_slice(&value.to_le_bytes());
        self.raw(&bytes);
    }

    pub(super) fn tagged_u32(&mut self, tag: u8, value: u32) {
        let mut bytes = [0; 5];
        bytes[0] = tag;
        bytes[1..].copy_from_slice(&value.to_le_bytes());
        self.raw(&bytes);
    }

    pub(super) fn box_fields(&mut self, box_node: &BoxNode) {
        self.scaled(box_node.width);
        self.scaled(box_node.height);
        self.scaled(box_node.depth);
        self.scaled(box_node.shift);
        self.i32(box_node.glue_set.numerator());
        self.i32(box_node.glue_set.denominator());
        self.u8(glue_sign_tag(box_node.glue_sign));
        self.u8(glue_order_tag(box_node.glue_order));
    }

    pub(super) fn glue_spec(&mut self, spec: GlueSpec) {
        self.scaled(spec.width);
        self.scaled(spec.stretch);
        self.u8(glue_order_tag(spec.stretch_order));
        self.scaled(spec.shrink);
        self.u8(glue_order_tag(spec.shrink_order));
    }
}
