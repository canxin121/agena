use crate::PluginInputFieldMetadata;

pub fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

pub fn schema_pointer_from_logical_path(prefix: &str, path: &str) -> Result<String, syn::Error> {
    let mut pointer = prefix.to_string();
    if path.trim().is_empty() {
        return Ok(pointer);
    }
    for raw_segment in path.split('.') {
        let mut segment = raw_segment;
        let mut array_depth = 0usize;
        while let Some(stripped) = segment.strip_suffix("[]") {
            array_depth += 1;
            segment = stripped;
        }
        if !segment.is_empty() {
            pointer.push_str("/properties/");
            pointer.push_str(&escape_json_pointer_segment(segment));
        }
        for _ in 0..array_depth {
            pointer.push_str("/items");
        }
    }
    Ok(pointer)
}

pub fn schema_relation_display_path(path: &str, metadata: &[PluginInputFieldMetadata]) -> String {
    if let Some(mapped) = metadata
        .iter()
        .find(|field| field.parse_path.value() == path)
        .map(|field| field.path.value())
    {
        return mapped;
    }
    let head_end = path.find('.').unwrap_or(path.len());
    let (head, tail) = path.split_at(head_end);
    let mut base = head;
    let mut suffix = String::new();
    while let Some(stripped) = base.strip_suffix("[]") {
        base = stripped;
        suffix.push_str("[]");
    }
    if let Some(mapped) = metadata
        .iter()
        .find(|field| field.parse_path.value() == base)
        .map(|field| field.path.value())
    {
        return format!("{mapped}{suffix}{tail}");
    }
    path.to_string()
}
