use js_sys::{Array, Object, Uint8Array};
use wasm_bindgen::JsValue;

use super::set;

pub(crate) fn render_update(update: &umber::RenderUpdate) -> Result<JsValue, JsValue> {
    match update {
        umber::RenderUpdate::Snapshot(document) => render_snapshot(&document.revision),
        umber::RenderUpdate::Patch(patch) => render_patch(patch),
    }
}

fn render_snapshot(revision: &umber::RenderRevision) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(&object, "kind", &JsValue::from_str("snapshot"))?;
    set(&object, "schemaVersion", &JsValue::from_f64(1.0))?;
    set(
        &object,
        "sessionId",
        &JsValue::from_str(&revision.session_id.hex()),
    )?;
    set(
        &object,
        "revision",
        &JsValue::from_f64(revision.revision as f64),
    )?;
    set(
        &object,
        "digest",
        &JsValue::from_str(&revision.digest.hex()),
    )?;
    set(&object, "title", &JsValue::from_str(&revision.title))?;
    set(&object, "language", &JsValue::from_str(&revision.language))?;
    let resources: JsValue = render_resources(&revision.resources)?.into();
    set(&object, "resources", &resources)?;
    let pages = Array::new();
    for page in &revision.pages {
        pages.push(&render_page(page)?);
    }
    set(&object, "pages", &pages)?;
    Ok(object.into())
}

fn render_patch(patch: &umber::PatchPlan) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(&object, "kind", &JsValue::from_str("patch"))?;
    set(&object, "schemaVersion", &JsValue::from_f64(1.0))?;
    set(
        &object,
        "sessionId",
        &JsValue::from_str(&patch.session_id.hex()),
    )?;
    set(
        &object,
        "baseRevision",
        &JsValue::from_f64(patch.base_revision as f64),
    )?;
    set(
        &object,
        "targetRevision",
        &JsValue::from_f64(patch.target_revision as f64),
    )?;
    set(
        &object,
        "beforeDigest",
        &JsValue::from_str(&patch.before_digest.hex()),
    )?;
    set(
        &object,
        "afterDigest",
        &JsValue::from_str(&patch.after_digest.hex()),
    )?;
    if let Some(title) = &patch.title {
        set(&object, "title", &JsValue::from_str(title))?;
    }
    if let Some(language) = &patch.language {
        set(&object, "language", &JsValue::from_str(language))?;
    }
    let additions: JsValue = render_resources(&patch.resource_additions)?.into();
    set(&object, "resourceAdditions", &additions)?;
    let releases = Array::new();
    for identity in &patch.resource_releases {
        releases.push(&JsValue::from_str(&hex(identity)));
    }
    set(&object, "resourceReleases", &releases)?;
    let operations = Array::new();
    for operation in &patch.operations {
        operations.push(&render_operation(operation)?);
    }
    set(&object, "operations", &operations)?;
    Ok(object.into())
}

fn render_resources(resources: &[umber::RenderResource]) -> Result<Array, JsValue> {
    let values = Array::new();
    for resource in resources {
        let value = Object::new();
        set(
            &value,
            "identity",
            &JsValue::from_str(&hex(&resource.identity)),
        )?;
        set(&value, "kind", &JsValue::from_str("font"))?;
        set(&value, "family", &JsValue::from_str(&resource.family))?;
        set(
            &value,
            "bytes",
            &Uint8Array::from(resource.bytes.as_slice()),
        )?;
        set(
            &value,
            "provenance",
            &JsValue::from_str(&resource.provenance),
        )?;
        values.push(&value);
    }
    Ok(values)
}

fn render_page(page: &umber::RenderPage) -> Result<JsValue, JsValue> {
    let object = render_page_header(&umber::RenderPageHeader::from(page))?;
    let nodes = Array::new();
    for node in &page.nodes {
        nodes.push(&render_node(node)?);
    }
    set(&object, "nodes", &nodes)?;
    Ok(object.into())
}

fn render_page_header(page: &umber::RenderPageHeader) -> Result<Object, JsValue> {
    let object = Object::new();
    set(&object, "key", &JsValue::from_str(&page.key.hex()))?;
    set(
        &object,
        "ordinal",
        &JsValue::from_f64(f64::from(page.ordinal)),
    )?;
    set(
        &object,
        "widthSp",
        &JsValue::from_f64(f64::from(page.width.raw())),
    )?;
    set(
        &object,
        "heightSp",
        &JsValue::from_f64(f64::from(page.height.raw())),
    )?;
    set(
        &object,
        "originXSp",
        &JsValue::from_f64(f64::from(page.origin_x.raw())),
    )?;
    set(
        &object,
        "originYSp",
        &JsValue::from_f64(f64::from(page.origin_y.raw())),
    )?;
    set(&object, "mag", &JsValue::from_f64(f64::from(page.mag)))?;
    Ok(object)
}

fn render_node(node: &umber::RenderNode) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(&object, "key", &JsValue::from_str(&node.key.hex()))?;
    match &node.value {
        umber::RenderNodeValue::Box(value) => {
            set(&object, "kind", &JsValue::from_str("box"))?;
            set(&object, "boxId", &JsValue::from_f64(f64::from(value.id)))?;
            set(
                &object,
                "boxKind",
                &JsValue::from_str(match value.kind {
                    umber::BoxKind::Horizontal => "hbox",
                    umber::BoxKind::Vertical => "vbox",
                }),
            )?;
            geometry(&object, value.x, value.y, value.width, value.height)?;
            scaled(&object, "baselineSp", value.baseline)?;
        }
        umber::RenderNodeValue::Rule(value) => {
            set(&object, "kind", &JsValue::from_str("rule"))?;
            geometry(&object, value.x, value.y, value.width, value.height)?;
            optional_string(&object, "color", value.color.as_deref())?;
        }
        umber::RenderNodeValue::Text(value) => {
            set(&object, "kind", &JsValue::from_str("text"))?;
            scaled(&object, "xSp", value.x)?;
            scaled(&object, "baselineSp", value.baseline)?;
            set(&object, "text", &JsValue::from_str(&value.text))?;
            set(&object, "family", &JsValue::from_str(&value.family))?;
            set(
                &object,
                "fontSizeSp",
                &JsValue::from_f64(f64::from(value.font.at_size_raw)),
            )?;
            let positions = Array::new();
            let exact_positions = value.positions.len() == value.units.len()
                && value.text.chars().count() == value.units.len();
            for position in if exact_positions {
                value.positions.as_slice()
            } else {
                std::slice::from_ref(&value.x)
            } {
                positions.push(&JsValue::from_f64(f64::from(position.raw())));
            }
            set(&object, "positionsSp", &positions)?;
            set(
                &object,
                "direction",
                &JsValue::from_str(match value.direction {
                    umber::RenderDirection::LeftToRight => "ltr",
                    umber::RenderDirection::RightToLeft => "rtl",
                }),
            )?;
            if let Some(script) = value.script {
                set(
                    &object,
                    "script",
                    &JsValue::from_str(&String::from_utf8_lossy(&script)),
                )?;
            }
            optional_string(&object, "language", value.language.as_deref())?;
            settings(&object, "features", &value.features)?;
            settings(&object, "variations", &value.variations)?;
            if let Some(line) = value.accessibility_line {
                set(
                    &object,
                    "accessibilityLine",
                    &JsValue::from_f64(f64::from(line)),
                )?;
            }
            optional_string(&object, "color", value.color.as_deref())?;
            optional_string(&object, "link", value.link.as_deref())?;
        }
        umber::RenderNodeValue::Special(value) => {
            set(&object, "kind", &JsValue::from_str("special"))?;
            scaled(&object, "xSp", value.x)?;
            scaled(&object, "ySp", value.y)?;
            set(&object, "class", &JsValue::from_str(&value.class))?;
            set(
                &object,
                "payloadHex",
                &JsValue::from_str(&hex(&value.payload)),
            )?;
            let (action, action_value) = match &value.action {
                umber::RenderSpecialAction::ColorPush(value) => {
                    ("color-push", Some(value.as_str()))
                }
                umber::RenderSpecialAction::ColorPop => ("color-pop", None),
                umber::RenderSpecialAction::LinkStart(value) => {
                    ("link-start", Some(value.as_str()))
                }
                umber::RenderSpecialAction::LinkEnd => ("link-end", None),
                umber::RenderSpecialAction::Destination(value) => {
                    ("destination", Some(value.as_str()))
                }
                umber::RenderSpecialAction::Inert => ("inert", None),
            };
            set(&object, "action", &JsValue::from_str(action))?;
            optional_string(&object, "actionValue", action_value)?;
        }
        umber::RenderNodeValue::MathStart(value) => {
            set(&object, "kind", &JsValue::from_str("math-start"))?;
            set(&object, "mathId", &JsValue::from_f64(f64::from(value.id)))?;
            scaled(&object, "xSp", value.x)?;
            scaled(&object, "baselineSp", value.baseline)?;
            scaled(&object, "widthSp", value.width)?;
            scaled(&object, "heightSp", value.height)?;
            scaled(&object, "depthSp", value.depth)?;
        }
        umber::RenderNodeValue::MathGlyph(value) => {
            let glyph = value.glyph;
            set(&object, "kind", &JsValue::from_str("math-glyph"))?;
            scaled(&object, "xSp", glyph.x)?;
            scaled(&object, "baselineSp", glyph.baseline)?;
            scaled(&object, "widthSp", glyph.width)?;
            scaled(&object, "heightSp", glyph.height)?;
            scaled(&object, "depthSp", glyph.depth)?;
            set(
                &object,
                "glyphId",
                &JsValue::from_f64(f64::from(glyph.glyph_id)),
            )?;
            set(&object, "ssty", &JsValue::from_f64(f64::from(glyph.ssty)))?;
            set(
                &object,
                "fontInstance",
                &JsValue::from_str(&hex(&glyph.font_instance.bytes())),
            )?;
            match &value.drawing {
                umber::RenderMathDrawing::Text {
                    scalar,
                    family,
                    font_size_raw,
                    variations,
                } => {
                    set(&object, "drawing", &JsValue::from_str("text"))?;
                    set(&object, "text", &JsValue::from_str(&scalar.to_string()))?;
                    set(&object, "family", &JsValue::from_str(family))?;
                    set(
                        &object,
                        "fontSizeSp",
                        &JsValue::from_f64(f64::from(*font_size_raw)),
                    )?;
                    settings(&object, "variations", variations)?;
                }
                umber::RenderMathDrawing::Outline {
                    path,
                    units_per_em,
                    font_size_raw,
                } => {
                    set(&object, "drawing", &JsValue::from_str("outline"))?;
                    set(&object, "path", &JsValue::from_str(path))?;
                    set(
                        &object,
                        "unitsPerEm",
                        &JsValue::from_f64(f64::from(*units_per_em)),
                    )?;
                    set(
                        &object,
                        "fontSizeSp",
                        &JsValue::from_f64(f64::from(*font_size_raw)),
                    )?;
                }
            }
        }
        umber::RenderNodeValue::MathRule(value) => {
            set(&object, "kind", &JsValue::from_str("math-rule"))?;
            geometry(&object, value.x, value.y, value.width, value.height)?;
        }
        umber::RenderNodeValue::MathEnd => {
            set(&object, "kind", &JsValue::from_str("math-end"))?;
        }
    }
    Ok(object.into())
}

fn render_operation(operation: &umber::PatchOp) -> Result<JsValue, JsValue> {
    let object = Object::new();
    match operation {
        umber::PatchOp::RemoveNode { page, key } => {
            set(&object, "kind", &JsValue::from_str("remove-node"))?;
            set(&object, "page", &JsValue::from_str(&page.hex()))?;
            set(&object, "key", &JsValue::from_str(&key.hex()))?;
        }
        umber::PatchOp::RemovePage { key } => {
            set(&object, "kind", &JsValue::from_str("remove-page"))?;
            set(&object, "key", &JsValue::from_str(&key.hex()))?;
        }
        umber::PatchOp::InsertPage { index, page } => {
            set(&object, "kind", &JsValue::from_str("insert-page"))?;
            set(&object, "index", &JsValue::from_f64(*index as f64))?;
            set(&object, "page", &render_page(page)?)?;
        }
        umber::PatchOp::MovePage { key, index } => {
            set(&object, "kind", &JsValue::from_str("move-page"))?;
            set(&object, "key", &JsValue::from_str(&key.hex()))?;
            set(&object, "index", &JsValue::from_f64(*index as f64))?;
        }
        umber::PatchOp::InsertNode { page, index, node } => {
            set(&object, "kind", &JsValue::from_str("insert-node"))?;
            set(&object, "page", &JsValue::from_str(&page.hex()))?;
            set(&object, "index", &JsValue::from_f64(*index as f64))?;
            set(&object, "node", &render_node(node)?)?;
        }
        umber::PatchOp::MoveNode { page, key, index } => {
            set(&object, "kind", &JsValue::from_str("move-node"))?;
            set(&object, "page", &JsValue::from_str(&page.hex()))?;
            set(&object, "key", &JsValue::from_str(&key.hex()))?;
            set(&object, "index", &JsValue::from_f64(*index as f64))?;
        }
        umber::PatchOp::UpdatePage(page) => {
            set(&object, "kind", &JsValue::from_str("update-page"))?;
            let page: JsValue = render_page_header(page)?.into();
            set(&object, "page", &page)?;
        }
        umber::PatchOp::UpdateNode { page, node } => {
            set(&object, "kind", &JsValue::from_str("update-node"))?;
            set(&object, "page", &JsValue::from_str(&page.hex()))?;
            set(&object, "node", &render_node(node)?)?;
        }
    }
    Ok(object.into())
}

fn geometry(
    object: &Object,
    x: tex_arith::Scaled,
    y: tex_arith::Scaled,
    width: tex_arith::Scaled,
    height: tex_arith::Scaled,
) -> Result<(), JsValue> {
    scaled(object, "xSp", x)?;
    scaled(object, "ySp", y)?;
    scaled(object, "widthSp", width)?;
    scaled(object, "heightSp", height)
}

fn scaled(object: &Object, name: &str, value: tex_arith::Scaled) -> Result<(), JsValue> {
    set(object, name, &JsValue::from_f64(f64::from(value.raw())))
}

fn optional_string(object: &Object, name: &str, value: Option<&str>) -> Result<(), JsValue> {
    if let Some(value) = value {
        set(object, name, &JsValue::from_str(value))?;
    }
    Ok(())
}

fn settings<T: Copy + Into<i64>>(
    object: &Object,
    name: &str,
    values: &[([u8; 4], T)],
) -> Result<(), JsValue> {
    let array = Array::new();
    for (tag, setting) in values {
        let value = Object::new();
        set(
            &value,
            "tag",
            &JsValue::from_str(&String::from_utf8_lossy(tag)),
        )?;
        set(
            &value,
            "value",
            &JsValue::from_f64((*setting).into() as f64),
        )?;
        array.push(&value);
    }
    set(object, name, &array)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(DIGITS[usize::from(byte >> 4)]));
        value.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    value
}
