//! Tool spec generation shared by input and plugin expansion.

use syn::parse::Parser;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, LitStr, Meta, Path, Result, Token, Type};

use crate::PluginInputFieldMetadata;

#[derive(Clone)]
/// Configuration of a tool spec.
pub struct ToolSpecConfig {
    pub tool: Option<LitStr>,
    pub before_help: Option<LitStr>,
    pub after_help: Option<LitStr>,
    pub summary: Option<LitStr>,
    pub help: Option<LitStr>,
    pub translations: Vec<LocalizedToolDocs>,
    pub normalize: Option<Path>,
    pub validate: Option<Path>,
    pub trim: Vec<LitStr>,
    pub trim_suffix: Vec<PathStringConstraint>,
    pub non_empty: Vec<LitStr>,
    pub non_empty_if_present: Vec<LitStr>,
    pub minimums: Vec<PathValueConstraint>,
    pub maximums: Vec<PathValueConstraint>,
    pub exclusive_minimums: Vec<PathValueConstraint>,
    pub exclusive_maximums: Vec<PathValueConstraint>,
    pub exactly_one_of: Vec<Vec<LitStr>>,
    pub at_least_one_of: Vec<Vec<LitStr>>,
    pub requires: Vec<PathPairConstraint>,
    pub conflicts_with: Vec<PathPairConstraint>,
    pub required_unless_present: Vec<PathPairConstraint>,
    pub forbid_substrings: Vec<PathStringsConstraint>,
    pub distinct_trimmed: Vec<LitStr>,
    pub distinct_trimmed_within: Vec<PathPairConstraint>,
    pub min_items: Vec<PathUsizeConstraint>,
    pub max_items: Vec<PathUsizeConstraint>,
    pub min_properties: Vec<PathUsizeConstraint>,
    pub max_properties: Vec<PathUsizeConstraint>,
    pub min_chars: Vec<PathUsizeConstraint>,
    pub max_chars: Vec<PathUsizeConstraint>,
    pub formats: Vec<PathStringConstraint>,
    pub patterns: Vec<PathStringConstraint>,
    pub choices: Vec<PathValuesConstraint>,
    pub input_field_metadata: Vec<PluginInputFieldMetadata>,
    pub tags: Vec<Expr>,
    pub streaming: bool,
    pub input_shape: Option<Type>,
    pub output_ty: Option<Type>,
}

pub fn empty_tool_spec_config() -> ToolSpecConfig {
    ToolSpecConfig {
        tool: None,
        before_help: None,
        after_help: None,
        summary: None,
        help: None,
        translations: Vec::new(),
        normalize: None,
        validate: None,
        trim: Vec::new(),
        trim_suffix: Vec::new(),
        non_empty: Vec::new(),
        non_empty_if_present: Vec::new(),
        minimums: Vec::new(),
        maximums: Vec::new(),
        exclusive_minimums: Vec::new(),
        exclusive_maximums: Vec::new(),
        exactly_one_of: Vec::new(),
        at_least_one_of: Vec::new(),
        requires: Vec::new(),
        conflicts_with: Vec::new(),
        required_unless_present: Vec::new(),
        forbid_substrings: Vec::new(),
        distinct_trimmed: Vec::new(),
        distinct_trimmed_within: Vec::new(),
        min_items: Vec::new(),
        max_items: Vec::new(),
        min_properties: Vec::new(),
        max_properties: Vec::new(),
        min_chars: Vec::new(),
        max_chars: Vec::new(),
        formats: Vec::new(),
        patterns: Vec::new(),
        choices: Vec::new(),
        input_field_metadata: Vec::new(),
        tags: Vec::new(),
        streaming: false,
        input_shape: None,
        output_ty: None,
    }
}

/// Localized UI copy attached to plugin and tool documentation. These values
/// never replace the stable English docs supplied to models.
#[derive(Clone)]
pub struct LocalizedToolDocs {
    pub locale: LitStr,
    pub before_help: Option<LitStr>,
    pub after_help: Option<LitStr>,
    pub summary: Option<LitStr>,
    pub help: Option<LitStr>,
}

struct LocalizedDocsArguments {
    locale: LitStr,
    fields: Punctuated<Meta, Token![,]>,
}

impl Parse for LocalizedDocsArguments {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let locale = input.parse::<LitStr>()?;
        let fields = if input.is_empty() {
            Punctuated::new()
        } else {
            input.parse::<Token![,]>()?;
            input.parse_terminated(Meta::parse, Token![,])?
        };
        Ok(Self { locale, fields })
    }
}

/// Parse `translations(locale("zh-CN", summary = "..."), ...)` metadata.
pub fn parse_localized_tool_docs(
    tokens: proc_macro2::TokenStream,
) -> Result<Vec<LocalizedToolDocs>> {
    let entries = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
    let mut translations = Vec::new();
    for entry in entries {
        let Meta::List(entry) = entry else {
            return Err(syn::Error::new_spanned(
                entry,
                "translations entries must use locale(\"tag\", field = \"text\")",
            ));
        };
        if !entry.path.is_ident("locale") {
            return Err(syn::Error::new_spanned(
                entry.path,
                "translations entries must use locale(\"tag\", field = \"text\")",
            ));
        }
        let parsed = syn::parse2::<LocalizedDocsArguments>(entry.tokens)?;
        let mut result = LocalizedToolDocs {
            locale: parsed.locale,
            before_help: None,
            after_help: None,
            summary: None,
            help: None,
        };
        for field in parsed.fields {
            let Meta::NameValue(field) = field else {
                return Err(syn::Error::new_spanned(
                    field,
                    "translation fields must use `name = \"text\"`",
                ));
            };
            let Some(name) = field.path.get_ident().map(ToString::to_string) else {
                return Err(syn::Error::new_spanned(
                    field.path,
                    "expected a translation field name",
                ));
            };
            let value = match field.value {
                Expr::Lit(value) => match value.lit {
                    syn::Lit::Str(value) => value,
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "translation text must be a string literal",
                        ));
                    }
                },
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "translation text must be a string literal",
                    ));
                }
            };
            let slot = match name.as_str() {
                "before_help" => &mut result.before_help,
                "after_help" => &mut result.after_help,
                "summary" => &mut result.summary,
                "help" => &mut result.help,
                _ => {
                    return Err(syn::Error::new_spanned(
                        field.path,
                        format!("unsupported translated documentation field '{name}'"),
                    ));
                }
            };
            if slot.replace(value).is_some() {
                return Err(syn::Error::new_spanned(
                    field.path,
                    format!("duplicate translated documentation field '{name}'"),
                ));
            }
        }
        if result.locale.value().trim().is_empty() {
            return Err(syn::Error::new_spanned(
                result.locale,
                "translation locale must not be empty",
            ));
        }
        if translations.iter().any(|existing: &LocalizedToolDocs| {
            existing
                .locale
                .value()
                .eq_ignore_ascii_case(&result.locale.value())
        }) {
            return Err(syn::Error::new_spanned(
                result.locale,
                "duplicate translation locale",
            ));
        }
        translations.push(result);
    }
    Ok(translations)
}

#[derive(Clone)]
/// Usize path constraint.
pub struct PathUsizeConstraint {
    pub path: LitStr,
    pub value: usize,
}

#[derive(Clone)]
/// Pair path constraint.
pub struct PathPairConstraint {
    pub left: LitStr,
    pub right: LitStr,
}

#[derive(Clone)]
/// Strings path constraint.
pub struct PathStringsConstraint {
    pub path: LitStr,
    pub values: Vec<LitStr>,
}

#[derive(Clone)]
/// Value path constraint.
pub struct PathValueConstraint {
    pub path: LitStr,
    pub value: Expr,
}

#[derive(Clone)]
/// Values path constraint.
pub struct PathValuesConstraint {
    pub path: LitStr,
    pub values: Vec<Expr>,
}

#[derive(Clone)]
/// String path constraint.
pub struct PathStringConstraint {
    pub path: LitStr,
    pub value: LitStr,
}

/// Source of schema constraints.
pub trait SchemaConstraintSource {
    fn non_empty(&self) -> &[LitStr];
    fn non_empty_if_present(&self) -> &[LitStr];
    fn minimums(&self) -> &[PathValueConstraint];
    fn maximums(&self) -> &[PathValueConstraint];
    fn exclusive_minimums(&self) -> &[PathValueConstraint];
    fn exclusive_maximums(&self) -> &[PathValueConstraint];
    fn min_items(&self) -> &[PathUsizeConstraint];
    fn max_items(&self) -> &[PathUsizeConstraint];
    fn min_properties(&self) -> &[PathUsizeConstraint];
    fn max_properties(&self) -> &[PathUsizeConstraint];
    fn min_chars(&self) -> &[PathUsizeConstraint];
    fn max_chars(&self) -> &[PathUsizeConstraint];
    fn formats(&self) -> &[PathStringConstraint];
    fn patterns(&self) -> &[PathStringConstraint];
    fn choices(&self) -> &[PathValuesConstraint];
    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &[]
    }
}

/// Source of schema relations.
pub trait SchemaRelationSource {
    fn exactly_one_of(&self) -> &[Vec<LitStr>];
    fn at_least_one_of(&self) -> &[Vec<LitStr>];
    fn requires(&self) -> &[PathPairConstraint];
    fn conflicts_with(&self) -> &[PathPairConstraint];
    fn required_unless_present(&self) -> &[PathPairConstraint];
    fn forbid_substrings(&self) -> &[PathStringsConstraint];
    fn distinct_trimmed(&self) -> &[LitStr];
    fn distinct_trimmed_within(&self) -> &[PathPairConstraint];
}

impl SchemaConstraintSource for ToolSpecConfig {
    fn non_empty(&self) -> &[LitStr] {
        &self.non_empty
    }

    fn non_empty_if_present(&self) -> &[LitStr] {
        &self.non_empty_if_present
    }

    fn minimums(&self) -> &[PathValueConstraint] {
        &self.minimums
    }

    fn maximums(&self) -> &[PathValueConstraint] {
        &self.maximums
    }

    fn exclusive_minimums(&self) -> &[PathValueConstraint] {
        &self.exclusive_minimums
    }

    fn exclusive_maximums(&self) -> &[PathValueConstraint] {
        &self.exclusive_maximums
    }

    fn min_items(&self) -> &[PathUsizeConstraint] {
        &self.min_items
    }

    fn max_items(&self) -> &[PathUsizeConstraint] {
        &self.max_items
    }

    fn min_properties(&self) -> &[PathUsizeConstraint] {
        &self.min_properties
    }

    fn max_properties(&self) -> &[PathUsizeConstraint] {
        &self.max_properties
    }

    fn min_chars(&self) -> &[PathUsizeConstraint] {
        &self.min_chars
    }

    fn max_chars(&self) -> &[PathUsizeConstraint] {
        &self.max_chars
    }

    fn formats(&self) -> &[PathStringConstraint] {
        &self.formats
    }

    fn patterns(&self) -> &[PathStringConstraint] {
        &self.patterns
    }

    fn choices(&self) -> &[PathValuesConstraint] {
        &self.choices
    }

    fn input_field_metadata(&self) -> &[PluginInputFieldMetadata] {
        &self.input_field_metadata
    }
}

impl SchemaRelationSource for ToolSpecConfig {
    fn exactly_one_of(&self) -> &[Vec<LitStr>] {
        &self.exactly_one_of
    }

    fn at_least_one_of(&self) -> &[Vec<LitStr>] {
        &self.at_least_one_of
    }

    fn requires(&self) -> &[PathPairConstraint] {
        &self.requires
    }

    fn conflicts_with(&self) -> &[PathPairConstraint] {
        &self.conflicts_with
    }

    fn required_unless_present(&self) -> &[PathPairConstraint] {
        &self.required_unless_present
    }

    fn forbid_substrings(&self) -> &[PathStringsConstraint] {
        &self.forbid_substrings
    }

    fn distinct_trimmed(&self) -> &[LitStr] {
        &self.distinct_trimmed
    }

    fn distinct_trimmed_within(&self) -> &[PathPairConstraint] {
        &self.distinct_trimmed_within
    }
}
