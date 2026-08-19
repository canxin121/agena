//! # agena-macro-core
//!
//! Ordinary-library implementation support shared by Agena proc-macro
//! entrypoints.
//!
//! All expansion logic for the `agena-plugin-sdk` macros lives here:
//! tool input shape/schema generation (`input_*` modules), plugin expansion
//! and manifest construction (`plugin_*` modules), and shared parsing/serde
//! helpers. The thin [`agena_macros`] crate re-exports the entrypoints as
//! `#[proc_macro]` items.

pub mod input_arg_output_support;
pub mod input_arg_parse_support;
pub mod input_arg_spec_support;
pub mod input_arg_support;
pub mod input_builtin_support;
pub mod input_config_parsing;
pub mod input_configs;
pub mod input_constraint_paths;
pub mod input_dispatch;
pub mod input_enum_shape;
pub mod input_expand_support;
pub mod input_path_support;
pub mod input_postprocess_support;
pub mod input_schema_metadata;
pub mod input_schema_metadata_fields;
pub mod input_schema_path_support;
pub mod input_shape_support;
pub mod macro_parse_support;
pub mod macro_type_support;
pub mod plugin_expand_support;
pub mod plugin_hooks;
pub mod plugin_hooks_support;
pub mod plugin_impl_config;
pub mod plugin_manifest;
pub mod plugin_method_arg_support;
pub mod plugin_method_support;
pub mod plugin_plan_support;
pub mod plugin_runtime;
pub mod plugin_settings_store;
pub mod plugin_tool_config;
pub mod plugin_tooling;
pub mod plugin_types;
pub mod serde_rename_support;
pub mod tool_spec_support;

// These re-exports preserve the original support crate's root namespace.  The
// support modules were split into this ordinary library, but many of them use
// `crate::...`/`super::...` references to symbols that used to be imported by
// `agena-macros/src/lib.rs`.
pub use input_arg_parse_support::parse_input_field_arg_attrs;
pub use input_arg_spec_support::apply_arg_config_to_spec;
pub use input_arg_support::{
    PluginInputFieldAliasSpec, PluginInputFieldDefaultSpec, PluginInputFieldMetadata,
    apply_input_field_arg_attrs, input_constraint_field_lookup, normalized_input_variant_config,
    prepare_input_field_names, validate_input_jsonpath_lit,
};
pub use input_builtin_support::{
    built_in_normalization_tokens, built_in_post_parse_normalization_tokens,
    built_in_validation_tokens,
};
pub use input_config_parsing::{parse_input_config, parse_input_variant_config};
pub use input_configs::{
    ToolInputConfig, ToolInputVariantConfig, input_variant_action_name, single_segment_ident,
};
pub use input_constraint_paths::{
    append_constraint_path_suffix, normalize_array_value_constraints,
    normalize_array_value_lit_paths, normalize_array_value_nested_path_constraints,
    prefixed_constraint_group, resolve_constraint_expr_paths, resolve_constraint_group_paths,
    resolve_constraint_lit_paths, resolve_constraint_pair_paths, resolve_constraint_string_paths,
    resolve_constraint_strings_paths, resolve_constraint_usize_paths,
    resolve_constraint_values_paths, resolve_known_constraint_path,
};
pub use input_dispatch::{dispatch_variant_pattern_and_args, expand_input_dispatch_fn};
pub use input_enum_shape::{
    expand_input_shape_enum_normalize_fn, expand_input_shape_enum_post_parse_normalize_expr,
    expand_input_shape_variant_validation_arm,
};
pub use input_path_support::{
    expand_flatten_shape_input_keys_expr, expand_flatten_shape_schema_normalize_expr,
    expand_input_networks_expr, expand_input_paths_expr, expand_input_shape_resolved_path_expr,
    expand_input_tags_expr, expand_nested_shape_input_keys_expr,
    expand_nested_shape_network_specs_expr, expand_nested_shape_path_specs_expr,
    expand_nested_shape_schema_normalize_expr, struct_flatten_shape_types,
    struct_nested_shape_fields,
};
pub use input_postprocess_support::{
    expand_input_alias_normalize_tokens, expand_input_default_insert_tokens,
    expand_input_default_schema_metadata_tokens, expand_input_example_expr,
    expand_input_root_default_insert_tokens, expand_input_root_default_schema_metadata_tokens,
    expand_input_root_example_schema_metadata_tokens,
    expand_input_shape_enum_parse_error_remap_expr,
    expand_input_shape_enum_validate_error_remap_expr, expand_result_path_remap_expr,
    input_default_schema_metadata_calls, input_error_path_mappings,
};
pub use input_schema_metadata::{
    constraint_relation_metadata_calls, constraint_schema_metadata_calls,
    expand_schema_metadata_fn, tool_spec_schema_metadata_calls,
};
pub use input_shape_support::{
    expand_flatten_shape_post_parse_tokens, expand_generated_input_post_parse_tokens,
    field_is_flatten, flatten_shape_type, named_field_object_insert_tokens,
};
pub use macro_parse_support::{
    default_operation_id, default_tool_name, doc_summary, doc_text, expr_array_lit_strs,
    expr_array_values, expr_lit_bool, expr_lit_i32, expr_lit_str, expr_lit_usize, expr_path,
    expr_string_like, ident_to_snake_case, lit_str_from_text, operation_title_from_id,
    parse_expr_list, parse_item_lit_str_list, parse_item_path_expr_constraint,
    parse_item_path_expr_list_constraint, parse_item_path_format_constraint,
    parse_item_path_lit_str_constraint, parse_item_path_pattern_constraint,
    parse_item_path_usize_constraint, parse_lit_str_list, parse_path_expr_constraint,
    parse_path_expr_list_constraint, parse_path_format_constraint, parse_path_lit_str_constraint,
    parse_path_lit_str_list_constraint, parse_path_pair_constraint, parse_path_pattern_constraint,
    parse_path_usize_constraint,
};
pub use macro_type_support::{
    input_type_semantic_shape, network_semantic_label, path_permission_kind_label,
    picker_kind_label, type_display, type_first_generic_arg, type_is_plugin_command_context,
    type_is_reference, type_is_tool_invoke_context, type_is_unit, type_last_segment_is,
    type_mentions_segment, type_without_reference, types_equivalent, validate_format_lit,
    validate_input_jsonpath, validate_pattern_lit,
};
pub use plugin_hooks::{PluginHookPlan, build_plugin_hook_plan};
pub use plugin_impl_config::{PluginImplConfig, expr_path_ident, parse_type_list};
pub use plugin_method_arg_support::{
    ArgAttrArgs, build_plugin_operation_input_plan, build_plugin_tool_method_shape,
    ensure_arg_permission_locator_has_semantic,
};
pub use plugin_method_support::{
    NestedInputShapeField, NestedInputShapeSpec, ensure_plugin_method_shared_receiver,
    generated_input_alias_specs, generated_input_flatten_shape_types,
    generated_input_nested_shape_fields, input_keys_for_parse_path, nested_input_shape_field,
    nested_input_shape_spec, nested_input_shape_spec_from_type, plugin_method_has_shared_receiver,
    plugin_method_return_type, plugin_method_return_value_type, plugin_method_tool_output,
    reject_duplicate_operation_plans, reject_duplicate_service_plans, reject_duplicate_tool_plans,
    stream_sink_is_edge_info, typed_arg_types, typed_arg_types_from_inputs,
};
pub use plugin_plan_support::{
    build_tool_operation_plan, expand_plugin_operation_usage_expr, operation_generated_input_model,
    parse_plugin_inherent_method_attrs, plugin_impl_method_infos,
};
pub use plugin_tool_config::parse_plugin_tool_method_attr;
pub use plugin_tooling::{expand_plugin_operation_definition, expand_plugin_tool_input_schema};
pub use plugin_types::{
    PluginArgConfig, PluginCallInput, PluginContextArg, PluginGeneratedInputField,
    PluginGeneratedToolInput, PluginInherentMethodAttrs, PluginInputNetworkSpec,
    PluginInputPathSpec, PluginMethodInfo, PluginNetworkSemantic, PluginOperationAttrArgs,
    PluginOperationHandlerPlan, PluginOperationInputPlan, PluginOperationMethodShape,
    PluginOperationPlan, PluginPathPermissionKind, PluginPickerKind, PluginServiceAttrArgs,
    PluginServiceAttrTarget, PluginServiceInputPlan, PluginServicePlan, PluginServiceTargetPlan,
    PluginToolAttrConfig, PluginToolInvokeHandler, PluginToolMethodShape,
    PluginToolNetworkPermissionRule, PluginToolOperationConfig, PluginToolOutputPlan,
    PluginToolPathPermissionRule, PluginToolPermissionHandlers, PluginToolPlan,
    PluginToolStreamHandler, PluginToolStreamSignature, plugin_attr_has_explicit_args,
};
pub use serde_rename_support::{
    SerdeRenameRule, field_has_serde_default, field_schema_aliases,
    field_schema_property_name_with_rule, serde_rename_all_fields_rule, serde_rename_all_rule,
};
pub use tool_spec_support::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, SchemaConstraintSource, SchemaRelationSource,
    ToolSpecConfig, empty_tool_spec_config,
};
