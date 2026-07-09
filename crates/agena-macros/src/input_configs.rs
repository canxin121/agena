use syn::{Expr, LitStr, Path, Result, Variant};

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, PluginInputFieldAliasSpec,
    PluginInputFieldDefaultSpec, PluginInputFieldMetadata, PluginInputNetworkSpec,
    PluginInputPathSpec, SchemaConstraintSource, SchemaRelationSource, ident_to_snake_case,
};

pub(crate) struct ToolInputVariantConfig {
    pub(crate) action: Option<LitStr>,
    pub(crate) validate: Option<Path>,
    pub(crate) handle: Option<Path>,
    pub(crate) handle_with_context: Option<Path>,
    pub(crate) stream_handle: Option<Path>,
    pub(crate) stream_handle_with_context: Option<Path>,
    pub(crate) permission_paths_handle: Option<Path>,
    pub(crate) permission_networks_handle: Option<Path>,
    pub(crate) handle_by_value: bool,
    pub(crate) trim: Vec<LitStr>,
    pub(crate) trim_suffix: Vec<PathStringConstraint>,
    pub(crate) non_empty: Vec<LitStr>,
    pub(crate) non_empty_if_present: Vec<LitStr>,
    pub(crate) minimums: Vec<PathValueConstraint>,
    pub(crate) maximums: Vec<PathValueConstraint>,
    pub(crate) exclusive_minimums: Vec<PathValueConstraint>,
    pub(crate) exclusive_maximums: Vec<PathValueConstraint>,
    pub(crate) exactly_one_of: Vec<Vec<LitStr>>,
    pub(crate) at_least_one_of: Vec<Vec<LitStr>>,
    pub(crate) requires: Vec<PathPairConstraint>,
    pub(crate) conflicts_with: Vec<PathPairConstraint>,
    pub(crate) required_unless_present: Vec<PathPairConstraint>,
    pub(crate) forbid_substrings: Vec<PathStringsConstraint>,
    pub(crate) distinct_trimmed: Vec<LitStr>,
    pub(crate) distinct_trimmed_within: Vec<PathPairConstraint>,
    pub(crate) min_items: Vec<PathUsizeConstraint>,
    pub(crate) max_items: Vec<PathUsizeConstraint>,
    pub(crate) min_properties: Vec<PathUsizeConstraint>,
    pub(crate) max_properties: Vec<PathUsizeConstraint>,
    pub(crate) min_chars: Vec<PathUsizeConstraint>,
    pub(crate) max_chars: Vec<PathUsizeConstraint>,
    pub(crate) formats: Vec<PathStringConstraint>,
    pub(crate) patterns: Vec<PathStringConstraint>,
    pub(crate) choices: Vec<PathValuesConstraint>,
    pub(crate) input_paths: Vec<PluginInputPathSpec>,
    pub(crate) input_networks: Vec<PluginInputNetworkSpec>,
    pub(crate) input_aliases: Vec<PluginInputFieldAliasSpec>,
    pub(crate) input_defaults: Vec<PluginInputFieldDefaultSpec>,
    pub(crate) input_field_metadata: Vec<PluginInputFieldMetadata>,
    pub(crate) default_when_empty: bool,
    pub(crate) infer_when_present: Vec<LitStr>,
    pub(crate) drop_keys: Vec<LitStr>,
}

pub(crate) struct ToolInputConfig {
    pub(crate) example: Option<Expr>,
    pub(crate) default: bool,
    pub(crate) default_expr: Option<Expr>,
    pub(crate) normalize: Option<Path>,
    pub(crate) validate: Option<Path>,
    pub(crate) handler_receiver: Option<Path>,
    pub(crate) handle: Option<Path>,
    pub(crate) handle_with_context: Option<Path>,
    pub(crate) stream_handle: Option<Path>,
    pub(crate) stream_handle_with_context: Option<Path>,
    pub(crate) permission_paths_handle: Option<Path>,
    pub(crate) permission_networks_handle: Option<Path>,
    pub(crate) handle_field: Option<Path>,
    pub(crate) handle_by_value: bool,
    pub(crate) trim: Vec<LitStr>,
    pub(crate) trim_suffix: Vec<PathStringConstraint>,
    pub(crate) non_empty: Vec<LitStr>,
    pub(crate) non_empty_if_present: Vec<LitStr>,
    pub(crate) minimums: Vec<PathValueConstraint>,
    pub(crate) maximums: Vec<PathValueConstraint>,
    pub(crate) exclusive_minimums: Vec<PathValueConstraint>,
    pub(crate) exclusive_maximums: Vec<PathValueConstraint>,
    pub(crate) exactly_one_of: Vec<Vec<LitStr>>,
    pub(crate) at_least_one_of: Vec<Vec<LitStr>>,
    pub(crate) requires: Vec<PathPairConstraint>,
    pub(crate) conflicts_with: Vec<PathPairConstraint>,
    pub(crate) required_unless_present: Vec<PathPairConstraint>,
    pub(crate) forbid_substrings: Vec<PathStringsConstraint>,
    pub(crate) distinct_trimmed: Vec<LitStr>,
    pub(crate) distinct_trimmed_within: Vec<PathPairConstraint>,
    pub(crate) min_items: Vec<PathUsizeConstraint>,
    pub(crate) max_items: Vec<PathUsizeConstraint>,
    pub(crate) min_properties: Vec<PathUsizeConstraint>,
    pub(crate) max_properties: Vec<PathUsizeConstraint>,
    pub(crate) min_chars: Vec<PathUsizeConstraint>,
    pub(crate) max_chars: Vec<PathUsizeConstraint>,
    pub(crate) formats: Vec<PathStringConstraint>,
    pub(crate) patterns: Vec<PathStringConstraint>,
    pub(crate) choices: Vec<PathValuesConstraint>,
    pub(crate) input_paths: Vec<PluginInputPathSpec>,
    pub(crate) input_networks: Vec<PluginInputNetworkSpec>,
    pub(crate) input_aliases: Vec<PluginInputFieldAliasSpec>,
    pub(crate) input_defaults: Vec<PluginInputFieldDefaultSpec>,
    pub(crate) input_field_metadata: Vec<PluginInputFieldMetadata>,
}

impl SchemaConstraintSource for ToolInputConfig {
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

impl SchemaRelationSource for ToolInputConfig {
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

impl SchemaConstraintSource for ToolInputVariantConfig {
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

impl SchemaRelationSource for ToolInputVariantConfig {
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

pub(crate) fn input_variant_action_name(
    variant: &Variant,
    config: &ToolInputVariantConfig,
) -> LitStr {
    config
        .action
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident_to_snake_case(&variant.ident), variant.ident.span()))
}

pub(crate) fn single_segment_ident(path: &Path, label: &str) -> Result<syn::Ident> {
    if path.leading_colon.is_none() && path.segments.len() == 1 {
        Ok(path.segments.first().expect("one segment").ident.clone())
    } else {
        Err(syn::Error::new_spanned(
            path,
            format!("{label} must be a single field identifier"),
        ))
    }
}
