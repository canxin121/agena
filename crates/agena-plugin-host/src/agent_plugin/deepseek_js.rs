//! Evaluate Cordis loader expressions without exposing Node or host services.

use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use rquickjs::{CatchResultExt, Context, Ctx, Runtime, Value, function::Func};
use serde_yaml::Value as Yaml;
use url::Url;
use yaml_rust2::scanner::{Scanner, Token, TokenType};

const URL_BINDINGS: &str = r#"
const resolveUrl = globalThis.agenaResolveUrl;
const fileUrlToPath = globalThis.agenaFileUrlToPath;
const pathToFileUrl = globalThis.agenaPathToFileUrl;
delete globalThis.agenaResolveUrl;
delete globalThis.agenaFileUrlToPath;
delete globalThis.agenaPathToFileUrl;
class URL {
    constructor(input, base) {
        const href = resolveUrl(String(input), base === undefined ? null : String(base));
        if (href == null) throw new TypeError('Invalid URL');
        this.href = href;
    }
    toString() { return this.href; }
}
const nodeUrl = Object.freeze({
    URL,
    fileURLToPath(input) {
        const path = fileUrlToPath(String(input));
        if (path == null) throw new TypeError('Invalid local file URL');
        return path;
    },
    pathToFileURL(input) {
        const href = pathToFileUrl(String(input));
        if (href == null) throw new TypeError('Invalid local file path');
        return new URL(href);
    },
});
"#;

pub(super) fn normalize_tags(text: &str) -> Result<String, String> {
    const FULL_TAG: &str = "!<tag:yaml.org,2002:js>";
    let mut scanner = Scanner::new(text.chars());
    let byte_offsets: Vec<_> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut ranges = Vec::new();
    for Token(mark, token) in &mut scanner {
        if let TokenType::Tag(handle, suffix) = token
            && ((handle == "!!" && suffix == "js")
                || (handle.is_empty() && suffix == "tag:yaml.org,2002:js"))
        {
            let start = *byte_offsets
                .get(mark.index())
                .ok_or("invalid YAML tag location")?;
            let tag = if handle == "!!" { "!!js" } else { FULL_TAG };
            if !text[start..].starts_with(tag) {
                return Err("invalid Cordis JavaScript tag".into());
            }
            ranges.push(start..start + tag.len());
        }
    }
    if let Some(error) = scanner.get_error() {
        return Err(format!("invalid Cordis patch YAML: {error}"));
    }
    let mut normalized = text.to_owned();
    for range in ranges.into_iter().rev() {
        normalized.replace_range(range, "!js");
    }
    Ok(normalized)
}

pub(super) fn with_loader_environment<T>(
    root: &Path,
    environment: &[String],
    evaluate: impl FnOnce(&Ctx<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let env: BTreeMap<_, _> = environment
        .iter()
        .filter_map(|name| super::mcp::read_var(name).ok().map(|value| (name, value)))
        .collect();
    let env = serde_json::to_string(&env).map_err(js_error)?;
    let base = Url::from_directory_path(root).map_err(|()| "invalid Cordis bundle directory")?;
    let base = serde_json::to_string(base.as_str()).map_err(js_error)?;
    let end = Instant::now() + Duration::from_secs(1);
    let runtime = create_runtime(move || Instant::now() >= end)?;
    with_context(&runtime, |ctx| {
        let platform = if cfg!(target_os = "windows") {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        bind_url_primitives(ctx)?;
        let setup = format!(
            r#"{URL_BINDINGS}
const baseUrl = {base};
const env = Object.freeze({env});
const process = Object.freeze({{
    env,
    platform: {platform:?},
    getBuiltinModule(name) {{ return name === 'node:url' || name === 'url' ? nodeUrl : undefined; }},
}});"#,
        );
        ctx.eval::<(), _>(setup).catch(ctx).map_err(js_error)?;
        evaluate(ctx)
    })
}

fn bind_url_primitives(ctx: &Ctx<'_>) -> Result<(), String> {
    let globals = ctx.globals();
    globals
        .set("agenaResolveUrl", Func::from(resolve_url))
        .map_err(js_error)?;
    globals
        .set("agenaFileUrlToPath", Func::from(file_url_to_path))
        .map_err(js_error)?;
    globals
        .set("agenaPathToFileUrl", Func::from(path_to_file_url))
        .map_err(js_error)
}

pub(super) fn install_module_url(ctx: &Ctx<'_>) -> Result<(), String> {
    bind_url_primitives(ctx)?;
    ctx.eval::<(), _>(format!(
        "{URL_BINDINGS}\nglobalThis.URL = URL;\nglobalThis.agenaCordisNodeUrl = nodeUrl;"
    ))
    .catch(ctx)
    .map_err(js_error)
}

pub(super) fn create_runtime(
    interrupt: impl FnMut() -> bool + Send + 'static,
) -> Result<Runtime, String> {
    let runtime = Runtime::new().map_err(js_error)?;
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(256 * 1024);
    runtime.set_interrupt_handler(Some(Box::new(interrupt)));
    Ok(runtime)
}

pub(super) fn with_context<T>(
    runtime: &Runtime,
    evaluate: impl FnOnce(&Ctx<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let context = Context::full(runtime).map_err(js_error)?;
    context.with(|ctx| evaluate(&ctx))
}

fn resolve_url(input: String, base: Option<String>) -> Option<String> {
    let base = match base {
        Some(base) => Some(Url::parse(&base).ok()?),
        None => None,
    };
    Url::options()
        .base_url(base.as_ref())
        .parse(&input)
        .ok()
        .map(Into::into)
}

fn file_url_to_path(input: String) -> Option<String> {
    let url = Url::parse(&input).ok()?;
    let escaped_path = url.path().to_ascii_lowercase();
    if escaped_path.contains("%2f") || (cfg!(windows) && escaped_path.contains("%5c")) {
        return None;
    }
    url.to_file_path().ok()?.into_os_string().into_string().ok()
}

fn path_to_file_url(input: String) -> Option<String> {
    let path = Path::new(&input);
    if !path.is_absolute() {
        return None;
    }
    Url::from_file_path(path).ok().map(Into::into)
}

pub(super) fn interpolate_value(ctx: &Ctx<'_>, value: &mut Yaml) -> Result<(), String> {
    interpolate(ctx, value, "")
}

pub(super) fn interpolate_storage_config(ctx: &Ctx<'_>, value: &mut Yaml) -> Result<(), String> {
    // An imported storage root is a namespace. Resolve the native home helper
    // to a stable identifier without depending on either installation path.
    const BINDINGS: &str = r#"
const dshHomePath = (...parts) => {
    if (parts.some(part => typeof part !== 'string')) throw new TypeError('dshHomePath requires path strings');
    const path = [];
    for (const part of parts.join('/').split(/[/\\]+/)) {
        if (part === '..') path.pop();
        else if (part && part !== '.') path.push(part);
    }
    return 'dsh-home:/' + path.join('/');
};
"#;
    interpolate(ctx, value, BINDINGS)
}

fn interpolate(ctx: &Ctx<'_>, value: &mut Yaml, bindings: &str) -> Result<(), String> {
    let expression = match value {
        Yaml::Tagged(value) if value.tag == "js" => Some(
            value
                .value
                .as_str()
                .ok_or("Cordis JavaScript tag needs a scalar expression")?,
        ),
        Yaml::Mapping(values) if values.len() == 1 => values.get("__jsExpr").and_then(Yaml::as_str),
        _ => None,
    };
    if let Some(expression) = expression {
        let expression = expression.trim().trim_end_matches(';');
        let source = format!("(() => {{\n{bindings}\nreturn (\n{expression}\n);\n}})()");
        let result: Value = ctx.eval(source).catch(ctx).map_err(js_error)?;
        let json = ctx
            .json_stringify(result)
            .catch(ctx)
            .map_err(js_error)?
            .ok_or("Cordis JavaScript expression did not return a JSON value")?;
        *value = serde_yaml::from_str(&json.to_string().map_err(js_error)?).map_err(js_error)?;
        return Ok(());
    }
    match value {
        Yaml::Mapping(values) => {
            for value in values.values_mut() {
                interpolate(ctx, value, bindings)?;
            }
        }
        Yaml::Sequence(values) => {
            for value in values {
                interpolate(ctx, value, bindings)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn js_error(error: impl std::fmt::Display) -> String {
    format!("Cordis JavaScript expression: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yaml_tags_are_translated_and_expressions_are_isolated() {
        let text = r#"
- note: "示例 !!js is a literal here"
  config: !!js "'value-' + (3 + 4)"
  boxed: {__jsExpr: "({allow: typeof require === 'undefined'})"}
  allowed: !<tag:yaml.org,2002:js> "Object.keys(process.env)"
"#;
        let mut patches: Vec<Yaml> = serde_yaml::from_str(&normalize_tags(text).unwrap()).unwrap();
        let root = tempfile::tempdir().unwrap();
        with_loader_environment(root.path(), &[], |ctx| {
            for patch in &mut patches {
                interpolate_value(ctx, patch)?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(patches[0]["config"].as_str(), Some("value-7"));
        assert_eq!(
            patches[0]["note"].as_str(),
            Some("示例 !!js is a literal here")
        );
        assert_eq!(patches[0]["boxed"]["allow"].as_bool(), Some(true));
        assert!(patches[0]["allowed"].as_sequence().unwrap().is_empty());
    }
}
