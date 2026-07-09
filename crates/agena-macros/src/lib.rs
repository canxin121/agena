use proc_macro::TokenStream;
use syn::{DeriveInput, ItemImpl, parse_macro_input};

mod input_arg_output_support;
mod input_arg_parse_support;
mod input_arg_spec_support;
mod input_arg_support;
mod input_builtin_support;
mod input_config_parsing;
mod input_configs;
mod input_constraint_paths;
mod input_dispatch;
mod input_enum_shape;
mod input_expand_support;
mod input_path_support;
mod input_postprocess_support;
mod input_schema_metadata;
mod input_schema_metadata_fields;
mod input_schema_path_support;
mod input_shape_support;
mod macro_parse_support;
mod macro_type_support;
mod plugin_config_store;
mod plugin_expand_support;
mod plugin_hooks;
mod plugin_hooks_support;
mod plugin_impl_config;
mod plugin_manifest;
mod plugin_method_arg_support;
mod plugin_method_support;
mod plugin_plan_support;
mod plugin_runtime;
mod plugin_tool_config;
mod plugin_tooling;
mod plugin_types;
mod serde_rename_support;
mod tool_spec_support;

use input_arg_parse_support::parse_input_field_arg_attrs;
use input_arg_spec_support::apply_arg_config_to_spec;
use input_arg_support::{
    PluginInputFieldAliasSpec, PluginInputFieldDefaultSpec, PluginInputFieldMetadata,
    apply_input_field_arg_attrs, input_constraint_field_lookup, normalized_input_variant_config,
    prepare_input_field_names, validate_input_jsonpath_lit,
};
use input_builtin_support::{
    built_in_normalization_tokens, built_in_post_parse_normalization_tokens,
    built_in_validation_tokens,
};
use input_config_parsing::{parse_input_config, parse_input_variant_config};
use input_configs::{
    ToolInputConfig, ToolInputVariantConfig, input_variant_action_name, single_segment_ident,
};
use input_constraint_paths::{
    append_constraint_path_suffix, normalize_array_value_constraints,
    normalize_array_value_lit_paths, normalize_array_value_nested_path_constraints,
    prefixed_constraint_group, resolve_constraint_expr_paths, resolve_constraint_group_paths,
    resolve_constraint_lit_paths, resolve_constraint_pair_paths, resolve_constraint_string_paths,
    resolve_constraint_strings_paths, resolve_constraint_usize_paths,
    resolve_constraint_values_paths, resolve_known_constraint_path,
};
use input_dispatch::{dispatch_variant_pattern_and_args, expand_input_dispatch_fn};
use input_enum_shape::{
    expand_input_shape_enum_normalize_fn, expand_input_shape_enum_post_parse_normalize_expr,
    expand_input_shape_variant_validation_arm,
};
use input_expand_support::expand_input;
use input_path_support::{
    expand_flatten_shape_input_keys_expr, expand_flatten_shape_schema_normalize_expr,
    expand_input_networks_expr, expand_input_paths_expr, expand_input_shape_resolved_path_expr,
    expand_input_tags_expr, expand_nested_shape_input_keys_expr,
    expand_nested_shape_network_specs_expr, expand_nested_shape_path_specs_expr,
    expand_nested_shape_schema_normalize_expr, struct_flatten_shape_types,
    struct_nested_shape_fields,
};
use input_postprocess_support::{
    expand_input_alias_normalize_tokens, expand_input_default_insert_tokens,
    expand_input_default_schema_metadata_tokens, expand_input_example_expr,
    expand_input_root_default_insert_tokens, expand_input_root_default_schema_metadata_tokens,
    expand_input_root_example_schema_metadata_tokens,
    expand_input_shape_enum_parse_error_remap_expr,
    expand_input_shape_enum_validate_error_remap_expr, expand_result_path_remap_expr,
    input_default_schema_metadata_calls, input_error_path_mappings,
};
use input_schema_metadata::{
    constraint_relation_metadata_calls, constraint_schema_metadata_calls,
    expand_schema_metadata_fn, tool_spec_schema_metadata_calls,
};
use input_shape_support::{
    expand_flatten_shape_post_parse_tokens, expand_generated_input_post_parse_tokens,
    field_is_flatten, flatten_shape_type, named_field_object_insert_tokens,
};
use macro_parse_support::{
    command_title_from_id, default_command_id, default_tool_name, doc_summary, doc_text,
    expr_array_lit_strs, expr_array_values, expr_lit_bool, expr_lit_i32, expr_lit_str,
    expr_lit_usize, expr_path, expr_string_like, ident_to_snake_case, lit_str_from_text,
    parse_expr_list, parse_item_lit_str_list, parse_item_path_expr_constraint,
    parse_item_path_expr_list_constraint, parse_item_path_format_constraint,
    parse_item_path_lit_str_constraint, parse_item_path_pattern_constraint,
    parse_item_path_usize_constraint, parse_lit_str_list, parse_path_expr_constraint,
    parse_path_expr_list_constraint, parse_path_format_constraint, parse_path_lit_str_constraint,
    parse_path_lit_str_list_constraint, parse_path_pair_constraint, parse_path_pattern_constraint,
    parse_path_usize_constraint,
};
use macro_type_support::{
    input_type_semantic_shape, network_semantic_label, path_permission_kind_label,
    picker_kind_label, type_display, type_first_generic_arg, type_is_plugin_command_context,
    type_is_reference, type_is_tool_invoke_context, type_is_unit, type_last_segment_is,
    type_mentions_segment, type_without_reference, types_equivalent, validate_format_lit,
    validate_input_jsonpath, validate_pattern_lit,
};
use plugin_config_store::expand_plugin_config_store;
use plugin_expand_support::expand_plugin_impl_attr;
pub(crate) use plugin_hooks::PluginHookPlan;
use plugin_hooks::build_plugin_hook_plan;
use plugin_impl_config::{PluginImplConfig, expr_path_ident, parse_type_list};
pub(crate) use plugin_method_arg_support::{
    ArgAttrArgs, build_plugin_command_input_plan, build_plugin_tool_method_shape,
    ensure_arg_permission_locator_has_semantic,
};
pub(crate) use plugin_method_support::{
    NestedInputShapeField, NestedInputShapeSpec, ensure_plugin_method_shared_receiver,
    generated_input_alias_specs, generated_input_flatten_shape_types,
    generated_input_nested_shape_fields, input_keys_for_parse_path, nested_input_shape_field,
    nested_input_shape_spec, nested_input_shape_spec_from_type, plugin_method_has_shared_receiver,
    plugin_method_return_type, plugin_method_return_value_type, plugin_method_tool_output,
    reject_duplicate_command_plans, reject_duplicate_tool_plans, stream_sink_is_edge_info,
    typed_arg_types, typed_arg_types_from_inputs,
};
pub(crate) use plugin_plan_support::{
    build_tool_command_plan, command_generated_input_model, expand_plugin_command_usage_expr,
    parse_plugin_inherent_method_attrs, plugin_impl_method_infos,
};
use plugin_tool_config::parse_plugin_tool_method_attr;
use plugin_tooling::expand_plugin_tool_input_schema;
pub(crate) use plugin_types::{
    PluginArgConfig, PluginCallInput, PluginCommandAttrArgs, PluginCommandHandlerPlan,
    PluginCommandInputPlan, PluginCommandMethodShape, PluginCommandPlan, PluginContextArg,
    PluginGeneratedInputField, PluginGeneratedToolInput, PluginInherentMethodAttrs,
    PluginInputNetworkSpec, PluginInputPathSpec, PluginMethodInfo, PluginNetworkSemantic,
    PluginPathPermissionKind, PluginPickerKind, PluginToolAttrConfig, PluginToolCommandConfig,
    PluginToolInvokeHandler, PluginToolMethodShape, PluginToolNetworkPermissionRule,
    PluginToolOutputPlan, PluginToolPathPermissionRule, PluginToolPermissionHandlers,
    PluginToolPlan, PluginToolStreamHandler, PluginToolStreamSignature,
    plugin_attr_has_explicit_args,
};
use serde_rename_support::{
    SerdeRenameRule, field_has_serde_default, field_schema_aliases,
    field_schema_property_name_with_rule, serde_rename_all_fields_rule, serde_rename_all_rule,
};
pub(crate) use tool_spec_support::{
    PathPairConstraint, PathStringConstraint, PathStringsConstraint, PathUsizeConstraint,
    PathValueConstraint, PathValuesConstraint, SchemaConstraintSource, SchemaRelationSource,
    ToolSpecConfig, empty_tool_spec_config,
};

#[proc_macro_attribute]
pub fn agena_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as proc_macro2::TokenStream);
    let item = parse_macro_input!(item as ItemImpl);
    match expand_plugin_impl_attr(attr, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(ToolInput, attributes(input, arg))]
pub fn derive_input(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_input(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(PluginConfigStore, attributes(config, plugin_config))]
pub fn derive_plugin_config_store(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_plugin_config_store(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
