use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use oxc::{
    allocator::Allocator,
    ast::ast::ImportExpression,
    ast_visit::Visit,
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer},
};
use rquickjs::{
    Ctx, Module,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use url::Url;

const MAX_MODULE_BYTES: u64 = 256 * 1024;
const MAX_MODULES: usize = 192;
const MAX_GRAPH_BYTES: usize = 4 * 1024 * 1024;
const EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];

pub(super) type ModuleSources = BTreeMap<String, Arc<str>>;

/// During validation the loader reads and compiles the graph. A session only
/// receives that snapshot, so cached packages and live reloads use one generation.
#[derive(Clone)]
pub(super) enum PackageModules {
    Discover {
        root: PathBuf,
        sources: Rc<RefCell<ModuleSources>>,
    },
    Snapshot(Arc<ModuleSources>),
}

impl PackageModules {
    fn contains(&self, url: &Url) -> bool {
        match self {
            Self::Discover { root, .. } => url
                .to_file_path()
                .ok()
                .and_then(|path| {
                    super::super::package_path(root, path.strip_prefix(root).ok()?).ok()
                })
                .is_some_and(|path| super::super::regular_file(&path)),
            Self::Snapshot(sources) => sources.contains_key(url.as_str()),
        }
    }

    pub(super) fn source(&self, name: &str) -> Result<Arc<str>, String> {
        match self {
            Self::Snapshot(sources) => sources
                .get(name)
                .cloned()
                .ok_or_else(|| "module is outside the validated graph".into()),
            Self::Discover { root, sources } => {
                let mut sources = sources.borrow_mut();
                if let Some(source) = sources.get(name) {
                    return Ok(source.clone());
                }
                if sources.len() >= MAX_MODULES {
                    return Err("Claude mod module graph exceeds its file limit".into());
                }
                let path = Url::parse(name)
                    .ok()
                    .and_then(|url| url.to_file_path().ok())
                    .ok_or("invalid Claude mod module path")?;
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "module is outside its plugin")?;
                if !is_module(relative) {
                    return Err("unsupported Claude mod extension".into());
                }
                let bytes = super::super::read_package_bytes(root, relative, MAX_MODULE_BYTES)?
                    .ok_or("Claude mod module does not exist")?;
                let source: Arc<str> = compile(relative, &bytes)?.into();
                if sources.values().map(|value| value.len()).sum::<usize>() + source.len()
                    > MAX_GRAPH_BYTES
                {
                    return Err("Claude mod module graph exceeds its byte limit".into());
                }
                sources.insert(name.to_owned(), source.clone());
                Ok(source)
            }
        }
    }
}

impl Resolver for PackageModules {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let failure = |message| rquickjs::Error::new_resolving_message(base, name, message);
        if attributes.is_some()
            || !(name.starts_with("./") || name.starts_with("../"))
            || name.contains(['?', '#', '\\'])
        {
            return Err(failure("Claude mods require package-relative imports"));
        }
        let base = Url::parse(base).map_err(|_| failure("invalid module URL"))?;
        let url = base
            .join(name)
            .map_err(|_| failure("invalid import path"))?;
        if !url.to_file_path().is_ok_and(|path| is_module(&path)) {
            // Claude's TypeScript examples leave the extension out. Try it
            // before an index lookup so ./name resolves to name.ts first.
            for stem in [
                name.to_owned(),
                format!("{}/index", name.trim_end_matches('/')),
            ] {
                for ext in EXTENSIONS {
                    if let Ok(candidate) = base.join(&format!("{stem}.{ext}"))
                        && self.contains(&candidate)
                    {
                        return Ok(candidate.into());
                    }
                }
            }
        }
        if self.contains(&url) {
            Ok(url.into())
        } else {
            Err(failure(
                "Claude mod import is missing or outside its package",
            ))
        }
    }
}

impl Loader for PackageModules {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let source = self
            .source(name)
            .map_err(|error| rquickjs::Error::new_loading_message(name, error))?;
        declare(ctx, name, &source)
    }
}

pub(super) fn declare<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    source: &str,
) -> rquickjs::Result<Module<'js, Declared>> {
    let module = Module::declare(ctx.clone(), name, source)?;
    module.meta()?.set("url", name)?;
    Ok(module)
}

pub(in crate::agent_plugin) fn is_module(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext))
}

#[derive(Default)]
struct DynamicImports(bool);

impl<'a> Visit<'a> for DynamicImports {
    fn visit_import_expression(&mut self, _: &ImportExpression<'a>) {
        self.0 = true;
    }
}

fn compile(path: &Path, input: &[u8]) -> Result<String, String> {
    let input = std::str::from_utf8(input).map_err(|_| "module must be UTF-8")?;
    let source_type = SourceType::from_path(path)
        .map_err(|error| error.to_string())?
        .with_module(true);
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, input, source_type).parse();
    if let Some(error) = parsed.diagnostics.first() {
        return Err(error.to_string());
    }
    let mut program = parsed.program;
    let mut imports = DynamicImports::default();
    imports.visit_program(&program);
    if imports.0 {
        return Err("Claude mod modules use static imports".into());
    }
    if source_type.is_javascript() && !source_type.is_jsx() {
        return Ok(input.to_owned());
    }
    let semantic = SemanticBuilder::new().build(&program);
    if let Some(error) = semantic.diagnostics.first() {
        return Err(error.to_string());
    }
    let options = TransformOptions {
        jsx: JsxOptions {
            runtime: JsxRuntime::Classic,
            jsx_plugin: source_type.is_jsx(),
            pragma: Some("h".into()),
            pragma_frag: Some("Fragment".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if let Some(error) = transformed.diagnostics.first() {
        return Err(error.to_string());
    }
    Ok(Codegen::new().build(&program).code)
}
