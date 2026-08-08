//! Tool spec generation shared by input and plugin expansion.

use syn::{Expr, LitStr, Path, Type};

use crate::{PluginInputFieldMetadata, PluginInputNetworkSpec, PluginInputPathSpec};

#[derive(Clone)]
/// Configuration of a tool spec.
pub struct ToolSpecConfig {
    pub tool: Option<LitStr>,
    pub before_help: Option<LitStr>,
    pub after_help: Option<LitStr>,
    pub summary: Option<LitStr>,
    pub help: Option<LitStr>,
    pub examples: Vec<LitStr>,
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
    pub input_paths: Vec<PluginInputPathSpec>,
    pub input_networks: Vec<PluginInputNetworkSpec>,
    pub input_field_metadata: Vec<PluginInputFieldMetadata>,
    pub tags: Vec<Expr>,
    pub capabilities: Vec<Expr>,
    pub concurrency_safe: bool,
    pub strict: bool,
    pub streaming: bool,
    pub mutating: bool,
    pub read_only: bool,
    pub shell: bool,
    pub interactive: bool,
    pub task: bool,
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
        tags: Vec::new(),
        capabilities: Vec::new(),
        concurrency_safe: false,
        strict: false,
        streaming: false,
        mutating: false,
        read_only: false,
        shell: false,
        interactive: false,
        task: false,
        input_shape: None,
        output_ty: None,
    }
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
