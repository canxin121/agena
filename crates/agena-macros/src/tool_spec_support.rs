use syn::{Expr, LitStr, Path, Type};

use crate::{PluginInputFieldMetadata, PluginInputNetworkSpec, PluginInputPathSpec};

#[derive(Clone)]
pub(crate) struct ToolSpecConfig {
    pub(crate) tool: Option<LitStr>,
    pub(crate) before_help: Option<LitStr>,
    pub(crate) after_help: Option<LitStr>,
    pub(crate) summary: Option<LitStr>,
    pub(crate) help: Option<LitStr>,
    pub(crate) examples: Vec<LitStr>,
    pub(crate) normalize: Option<Path>,
    pub(crate) validate: Option<Path>,
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
    pub(crate) input_field_metadata: Vec<PluginInputFieldMetadata>,
    pub(crate) display: Option<LitStr>,
    pub(crate) ui_display: Option<LitStr>,
    pub(crate) description_mode: Option<LitStr>,
    pub(crate) ui_display_mode: Option<LitStr>,
    pub(crate) tags: Vec<Expr>,
    pub(crate) capabilities: Vec<Expr>,
    pub(crate) concurrency_safe: bool,
    pub(crate) strict: bool,
    pub(crate) streaming: bool,
    pub(crate) input_shape: Option<Type>,
    pub(crate) output_ty: Option<Type>,
}

pub(crate) fn empty_tool_spec_config() -> ToolSpecConfig {
    ToolSpecConfig {
        tool: None,
        before_help: None,
        after_help: None,
        summary: None,
        help: None,
        examples: Vec::new(),
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
        input_paths: Vec::new(),
        input_networks: Vec::new(),
        input_field_metadata: Vec::new(),
        display: None,
        ui_display: None,
        description_mode: None,
        ui_display_mode: None,
        tags: Vec::new(),
        capabilities: Vec::new(),
        concurrency_safe: false,
        strict: false,
        streaming: false,
        input_shape: None,
        output_ty: None,
    }
}

#[derive(Clone)]
pub(crate) struct PathUsizeConstraint {
    pub(crate) path: LitStr,
    pub(crate) value: usize,
}

#[derive(Clone)]
pub(crate) struct PathPairConstraint {
    pub(crate) left: LitStr,
    pub(crate) right: LitStr,
}

#[derive(Clone)]
pub(crate) struct PathStringsConstraint {
    pub(crate) path: LitStr,
    pub(crate) values: Vec<LitStr>,
}

#[derive(Clone)]
pub(crate) struct PathValueConstraint {
    pub(crate) path: LitStr,
    pub(crate) value: Expr,
}

#[derive(Clone)]
pub(crate) struct PathValuesConstraint {
    pub(crate) path: LitStr,
    pub(crate) values: Vec<Expr>,
}

#[derive(Clone)]
pub(crate) struct PathStringConstraint {
    pub(crate) path: LitStr,
    pub(crate) value: LitStr,
}

pub(crate) trait SchemaConstraintSource {
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

pub(crate) trait SchemaRelationSource {
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
