//! Configuration values attached to tool inputs.

use syn::{Expr, LitStr, Path, Result, Variant};

use super::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, PluginInputFieldAliasSpec,
    PluginInputFieldDefaultSpec, PluginInputFieldMetadata, PluginInputNetworkSpec,
    PluginInputPathSpec, SchemaConstraintSource, SchemaRelationSource, ident_to_snake_case,
};

/// Configuration of one tool input variant.
pub struct ToolInputVariantConfig {
    pub action: Option<LitStr>,
    pub validate: Option<Path>,
    pub handle: Option<Path>,
    pub handle_with_context: Option<Path>,
    pub stream_handle: Option<Path>,
    pub stream_handle_with_context: Option<Path>,
    pub permission_paths_handle: Option<Path>,
    pub permission_networks_handle: Option<Path>,
    pub handle_by_value: bool,
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
    pub input_paths: Vec<PluginInputPathSpec>,
    pub input_networks: Vec<PluginInputNetworkSpec>,
    pub input_aliases: Vec<PluginInputFieldAliasSpec>,
    pub input_defaults: Vec<PluginInputFieldDefaultSpec>,
    pub input_field_metadata: Vec<PluginInputFieldMetadata>,
    pub default_when_empty: bool,
    pub infer_when_present: Vec<LitStr>,
    pub drop_keys: Vec<LitStr>,
}

/// Configuration of a tool input.
pub struct ToolInputConfig {
    pub example: Option<Expr>,
    pub default: bool,
    pub default_expr: Option<Expr>,
    pub normalize: Option<Path>,
    pub validate: Option<Path>,
    pub handler_receiver: Option<Path>,
    pub handle: Option<Path>,
    pub handle_with_context: Option<Path>,
    pub stream_handle: Option<Path>,
    pub stream_handle_with_context: Option<Path>,
    pub permission_paths_handle: Option<Path>,
    pub permission_networks_handle: Option<Path>,
    pub handle_field: Option<Path>,
    pub handle_by_value: bool,
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
    pub input_paths: Vec<PluginInputPathSpec>,
    pub input_networks: Vec<PluginInputNetworkSpec>,
    pub input_aliases: Vec<PluginInputFieldAliasSpec>,
    pub input_defaults: Vec<PluginInputFieldDefaultSpec>,
    pub input_field_metadata: Vec<PluginInputFieldMetadata>,
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

pub fn input_variant_action_name(variant: &Variant, config: &ToolInputVariantConfig) -> LitStr {
    config
        .action
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident_to_snake_case(&variant.ident), variant.ident.span()))
}

pub fn single_segment_ident(path: &Path, label: &str) -> Result<syn::Ident> {
    if path.leading_colon.is_none() && path.segments.len() == 1 {
        Ok(path.segments.first().expect("one segment").ident.clone())
    } else {
        Err(syn::Error::new_spanned(
            path,
            format!("{label} must be a single field identifier"),
        ))
    }
}
