use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct DiscriminatedSchemaVariant {
    pub(crate) field: String,
    pub(crate) value: String,
    pub(crate) schema: Value,
}

pub(crate) fn top_level_discriminated_variants(
    schema: &Value,
) -> Option<Vec<DiscriminatedSchemaVariant>> {
    let object = schema.as_object()?;
    let variants = ["oneOf", "anyOf", "allOf"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_array))?;
    if variants.len() <= 1 {
        return None;
    }

    let variant_objects = variants
        .iter()
        .map(Value::as_object)
        .collect::<Option<Vec<_>>>()?;
    let discriminant = variant_objects
        .iter()
        .fold(None::<BTreeSet<String>>, |candidates, variant| {
            let fields = variant
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| {
                    properties
                        .iter()
                        .filter_map(|(name, property)| {
                            let literals = string_literals(property)?;
                            (literals.len() == 1).then_some(name.clone())
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            Some(match candidates {
                Some(existing) => existing
                    .intersection(&fields)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                None => fields,
            })
        })
        .and_then(|candidates| {
            ["action", "target"]
                .into_iter()
                .find_map(|preferred| candidates.contains(preferred).then_some(preferred))
                .map(ToOwned::to_owned)
                .or_else(|| candidates.into_iter().next())
        })?;

    let mut seen_values = BTreeSet::new();
    let mut expanded = Vec::with_capacity(variant_objects.len());
    for variant in variant_objects {
        let value = variant
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(discriminant.as_str()))
            .and_then(string_literals)
            .and_then(|literals| literals.into_iter().next())?;
        if !seen_values.insert(value.clone()) {
            return None;
        }
        expanded.push(DiscriminatedSchemaVariant {
            field: discriminant.clone(),
            value,
            schema: strip_discriminant_from_variant(variant, discriminant.as_str()),
        });
    }

    Some(expanded)
}

pub(crate) fn top_level_union_variants(schema: &Value) -> Option<&[Value]> {
    let object = schema.as_object()?;
    ["oneOf", "anyOf", "allOf"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_array))
        .map(Vec::as_slice)
        .filter(|items| !items.is_empty())
}

pub(crate) fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

pub(crate) fn unescape_json_pointer_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

pub(crate) fn schema_order_key(schema: &Value) -> Option<String> {
    schema
        .get("x-agena-order")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty())
}

pub(crate) fn ordered_schema_properties<'a>(
    root: &'a Value,
    schema: &'a Value,
) -> Option<Vec<(&'a String, &'a Value)>> {
    let schema = resolve_schema_value(root, schema);
    let properties = schema.as_object()?.get("properties").and_then(Value::as_object)?;
    let mut ordered = properties.iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_name, left_property), (right_name, right_property)| {
        let left_order = schema_order_key(resolve_schema_value(root, left_property));
        let right_order = schema_order_key(resolve_schema_value(root, right_property));
        match (left_order, right_order) {
            (Some(left_order), Some(right_order)) => left_order
                .cmp(&right_order)
                .then_with(|| left_name.cmp(right_name)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left_name.cmp(right_name),
        }
    });
    Some(ordered)
}

pub(crate) fn resolve_schema_value<'a>(root: &'a Value, current: &'a Value) -> &'a Value {
    let mut current = current;
    while let Some((target, _)) = resolve_schema_ref(root, current) {
        current = target;
    }
    current
}

pub(crate) fn string_literals(value: &Value) -> Option<BTreeSet<String>> {
    let object = value.as_object()?;
    if let Some(value) = object.get("const").and_then(Value::as_str) {
        return Some(BTreeSet::from([value.to_owned()]));
    }
    object
        .get("enum")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}

pub(crate) fn resolve_schema_ref<'a>(root: &'a Value, current: &'a Value) -> Option<(&'a Value, &'a str)> {
    let target_pointer = current.get("$ref").and_then(Value::as_str)?.strip_prefix('#')?;
    let target = root.pointer(target_pointer)?;
    Some((target, target_pointer))
}

fn strip_discriminant_from_variant(variant: &serde_json::Map<String, Value>, field: &str) -> Value {
    let mut stripped = variant.clone();
    if let Some(properties) = stripped
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.remove(field);
    }
    if let Some(required) = stripped.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|item| item.as_str() != Some(field));
        if required.is_empty() {
            stripped.remove("required");
        }
    }
    stripped
        .entry("type".to_string())
        .or_insert_with(|| Value::String("object".to_string()));
    stripped
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    stripped.insert(
        "x-agena-discriminant-field".to_string(),
        Value::String(field.to_string()),
    );
    Value::Object(stripped)
}
