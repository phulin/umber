use js_sys::{Array, Object};
use umber::ResourceRequest;
use wasm_bindgen::JsValue;

use super::{set, typed_array};

pub(super) fn resource_requests(requests: Vec<ResourceRequest>) -> Result<Array, JsValue> {
    let result = Array::new();
    for request in requests {
        let object = Object::new();
        match request {
            ResourceRequest::File(request) => {
                set(&object, "type", &JsValue::from_str("file"))?;
                set(
                    &object,
                    "domain",
                    &JsValue::from_str(request.key().domain().wire_name()),
                )?;
                set(
                    &object,
                    "kind",
                    &JsValue::from_str(request.key().kind().wire_name()),
                )?;
                set(&object, "name", &JsValue::from_str(request.key().name()))?;
                set(
                    &object,
                    "originalName",
                    &JsValue::from_str(request.original_name()),
                )?;
            }
            ResourceRequest::Font(request) => {
                set(&object, "type", &JsValue::from_str("font"))?;
                set(
                    &object,
                    "logicalName",
                    &JsValue::from_str(request.key.logical_name()),
                )?;
                set(
                    &object,
                    "faceIndex",
                    &JsValue::from_f64(f64::from(request.key.face_index)),
                )?;
                let variations = Array::new();
                for coordinate in request.key.variation.coordinates() {
                    let value = Object::new();
                    set(
                        &value,
                        "tag",
                        &JsValue::from_str(&coordinate.tag.to_string()),
                    )?;
                    set(
                        &value,
                        "value",
                        &JsValue::from_f64(f64::from(coordinate.value)),
                    )?;
                    variations.push(&value);
                }
                set(&object, "variations", &variations)?;
                match request.key.variation.instance() {
                    umber::VariationInstance::Default => {
                        set(&object, "variationInstance", &JsValue::from_str("default"))?;
                    }
                    umber::VariationInstance::Coordinates => {
                        set(
                            &object,
                            "variationInstance",
                            &JsValue::from_str("coordinates"),
                        )?;
                    }
                    umber::VariationInstance::Named(name_id) => {
                        let instance = Object::new();
                        set(
                            &instance,
                            "namedNameId",
                            &JsValue::from_f64(f64::from(name_id)),
                        )?;
                        set(&object, "variationInstance", &instance)?;
                    }
                }
                let features = Array::new();
                for setting in request.key.feature_policy.settings() {
                    let value = Object::new();
                    set(&value, "tag", &JsValue::from_str(&setting.tag.to_string()))?;
                    set(
                        &value,
                        "value",
                        &JsValue::from_f64(f64::from(setting.value)),
                    )?;
                    features.push(&value);
                }
                set(&object, "features", &features)?;
                set(
                    &object,
                    "direction",
                    &JsValue::from_str(match request.key.direction {
                        umber::WritingDirection::LeftToRight => "ltr",
                        umber::WritingDirection::RightToLeft => "rtl",
                    }),
                )?;
                if let Some(script) = request.key.script {
                    set(&object, "script", &JsValue::from_str(&script.to_string()))?;
                }
                if let Some(language) = &request.key.language {
                    set(&object, "language", &JsValue::from_str(language.as_str()))?;
                }
                let accepted = Array::new();
                if request
                    .accepted_containers
                    .contains(umber::FontContainer::Woff2)
                {
                    accepted.push(&JsValue::from_str("woff2"));
                }
                set(&object, "acceptedContainers", &accepted)?;
            }
            ResourceRequest::PkFont(request) => {
                set(&object, "type", &JsValue::from_str("pk-font"))?;
                set(&object, "texName", &typed_array(request.tex_name()))?;
                set(&object, "dpi", &JsValue::from_f64(f64::from(request.dpi())))?;
                set(&object, "mode", &typed_array(request.mode()))?;
            }
        }
        result.push(&object);
    }
    Ok(result)
}
