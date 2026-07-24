use super::super::{
    ConfigDiagnostic, ConfigGroupLayout, ConfigGroupView, ConfigOverviewCard, ConfigSectionBody,
    ConfigSectionView, DiagnosticSeverity, JsonValue, PathSegment, PluginWorkbenchPlugin,
    compact_duration_summary, format_bytes_summary, get_value_at_path, ordered_object_keys,
    override_leaf_count, push_diag, title_for_config_path,
};
use super::{
    build_bool_row, build_generic_overview_section, build_generic_section, build_integer_row,
    build_nullable_string_row, build_pair_integer_row, config_path, section_issue_label,
    web_form_section,
};

pub(crate) fn plugin_semantic_diagnostics(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigDiagnostic> {
    if plugin.plugin_id != "agena.web" {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for (path, label) in [
        (
            config_path(["fetch", "request", "delay_ms"]),
            "fetch.request.delay_ms",
        ),
        (
            config_path(["fetch", "request", "timeout_secs"]),
            "fetch.request.timeout_secs",
        ),
        (
            config_path(["fetch", "request", "max_body_bytes"]),
            "fetch.request.max_body_bytes",
        ),
        (
            config_path(["fetch", "cache", "ttl_secs"]),
            "fetch.cache.ttl_secs",
        ),
        (
            config_path(["fetch", "cache", "capacity"]),
            "fetch.cache.capacity",
        ),
        (
            config_path(["crawl", "defaults", "max_pages"]),
            "crawl.defaults.max_pages",
        ),
        (
            config_path(["crawl", "limits", "max_pages"]),
            "crawl.limits.max_pages",
        ),
        (
            config_path(["crawl", "limits", "max_depth"]),
            "crawl.limits.max_depth",
        ),
        (
            config_path(["crawl", "indexing", "document_cache_ttl_secs"]),
            "crawl.indexing.document_cache_ttl_secs",
        ),
        (
            config_path(["crawl", "indexing", "chunk_chars"]),
            "crawl.indexing.chunk_chars",
        ),
        (
            config_path(["crawl", "indexing", "near_duplicate_hamming_distance"]),
            "crawl.indexing.near_duplicate_hamming_distance",
        ),
        (
            config_path(["search", "default_limit"]),
            "search.default_limit",
        ),
        (config_path(["search", "max_limit"]), "search.max_limit"),
        (
            config_path(["store", "retention", "max_documents"]),
            "store.retention.max_documents",
        ),
        (
            config_path(["store", "retention", "max_bytes"]),
            "store.retention.max_bytes",
        ),
        (
            config_path(["browser", "wait", "timeout_secs"]),
            "browser.wait.timeout_secs",
        ),
    ] {
        if get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_u64)
            .is_some_and(|value| value == 0)
        {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &path,
                &title_for_config_path(plugin, &path, label),
                "must be greater than 0",
            );
        }
    }
    for (left_path, right_path, message) in [
        (
            config_path(["crawl", "defaults", "max_pages"]),
            config_path(["crawl", "limits", "max_pages"]),
            "default value must be <= limit",
        ),
        (
            config_path(["crawl", "defaults", "max_depth"]),
            config_path(["crawl", "limits", "max_depth"]),
            "default value must be <= limit",
        ),
        (
            config_path(["search", "default_limit"]),
            config_path(["search", "max_limit"]),
            "default value must be <= max",
        ),
        (
            config_path(["store", "listing", "default_limit"]),
            config_path(["store", "listing", "max_limit"]),
            "default value must be <= max",
        ),
    ] {
        let Some(left) =
            get_value_at_path(&plugin.draft_config, &left_path).and_then(JsonValue::as_u64)
        else {
            continue;
        };
        let Some(right) =
            get_value_at_path(&plugin.draft_config, &right_path).and_then(JsonValue::as_u64)
        else {
            continue;
        };
        if left > right {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &left_path,
                &title_for_config_path(plugin, &left_path, "Value"),
                message,
            );
        }
    }
    for (path, message) in [
        (
            config_path(["browser", "executable_path"]),
            "executable path cannot be empty when set",
        ),
        (
            config_path(["browser", "wait", "for_selector"]),
            "selector cannot be empty when set",
        ),
    ] {
        if get_value_at_path(&plugin.draft_config, &path)
            .and_then(JsonValue::as_str)
            .is_some_and(|value| value.trim().is_empty())
        {
            push_diag(
                &mut diagnostics,
                DiagnosticSeverity::Error,
                &path,
                &title_for_config_path(plugin, &path, "Value"),
                message,
            );
        }
    }
    diagnostics
}

pub(crate) fn build_config_sections(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigSectionView> {
    if plugin.plugin_id == "agena.web" {
        build_web_config_sections(plugin)
    } else {
        build_generic_config_sections(plugin)
    }
}

pub(crate) fn build_web_config_sections(plugin: &PluginWorkbenchPlugin) -> Vec<ConfigSectionView> {
    let fetch_enabled = get_value_at_path(&plugin.draft_config, &config_path(["fetch", "enabled"]))
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let mut sections = vec![ConfigSectionView {
        key: "overview".to_owned(),
        title: "Overview".to_owned(),
        issue_count: plugin
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count(),
        dirty: plugin.dirty,
        body: ConfigSectionBody::Overview {
            cards: vec![
                ConfigOverviewCard {
                    title: "Fetch".to_owned(),
                    summary: format!(
                        "{}, {}, {}",
                        if fetch_enabled { "enabled" } else { "disabled" },
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["fetch", "request", "delay_ms"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "ms",
                            "delay",
                        ),
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["fetch", "request", "timeout_secs"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "s",
                            "timeout",
                        )
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["fetch"])),
                },
                ConfigOverviewCard {
                    title: "Crawl".to_owned(),
                    summary: format!(
                        "{} / depth {}, limit {} / {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "defaults", "max_pages"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "defaults", "max_depth"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "limits", "max_pages"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["crawl", "limits", "max_depth"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["crawl"])),
                },
                ConfigOverviewCard {
                    title: "Search".to_owned(),
                    summary: format!(
                        "default {}, max {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["search", "default_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["search", "max_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["search"])),
                },
                ConfigOverviewCard {
                    title: "Store".to_owned(),
                    summary: format!(
                        "{} docs, {}, listing {} / {}",
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "retention", "max_documents"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        format_bytes_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["store", "retention", "max_bytes"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default()
                        ),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "listing", "default_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                        get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["store", "listing", "max_limit"]),
                        )
                        .and_then(JsonValue::as_u64)
                        .unwrap_or_default(),
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["store"])),
                },
                ConfigOverviewCard {
                    title: "Browser".to_owned(),
                    summary: format!(
                        "{}, wait {}, selector {}",
                        if get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["browser", "enabled"]),
                        )
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false)
                        {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        compact_duration_summary(
                            get_value_at_path(
                                &plugin.draft_config,
                                &config_path(["browser", "wait", "timeout_secs"]),
                            )
                            .and_then(JsonValue::as_u64)
                            .unwrap_or_default(),
                            "s",
                            "",
                        ),
                        if get_value_at_path(
                            &plugin.draft_config,
                            &config_path(["browser", "wait", "for_selector"]),
                        )
                        .and_then(JsonValue::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                        {
                            "set"
                        } else {
                            "not set"
                        }
                    ),
                    issue_label: section_issue_label(plugin, &config_path(["browser"])),
                },
            ],
            lines: vec![
                format!(
                    "Schema             {}",
                    if plugin.schema_missing {
                        "Missing"
                    } else {
                        "Available"
                    }
                ),
                "Effective mode     Full config values".to_owned(),
                format!(
                    "Changed            {} field(s)",
                    override_leaf_count(&plugin.draft_override)
                ),
                format!(
                    "Diagnostics        {}",
                    if plugin.diagnostics.is_empty() {
                        "No issues".to_owned()
                    } else {
                        format!("{} issue(s)", plugin.diagnostics.len())
                    }
                ),
            ],
        },
    }];

    sections.push(web_form_section(
        plugin,
        "fetch",
        "Fetch",
        config_path(["fetch"]),
        (!fetch_enabled).then(|| {
            "Fetch disabled: agena.web/fetch and agena.web/crawl will be unavailable".to_owned()
        }),
        vec![
            ConfigGroupView {
                title: "Fetch".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![build_bool_row(
                    plugin,
                    "Enabled",
                    config_path(["fetch", "enabled"]),
                    None,
                )],
            },
            ConfigGroupView {
                title: "Request".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Delay",
                        config_path(["fetch", "request", "delay_ms"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Timeout",
                        config_path(["fetch", "request", "timeout_secs"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Max body size",
                        config_path(["fetch", "request", "max_body_bytes"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_bool_row(
                        plugin,
                        "Respect robots.txt",
                        config_path(["fetch", "request", "respect_robots_txt"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                ],
            },
            ConfigGroupView {
                title: "Cache".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "TTL",
                        config_path(["fetch", "cache", "ttl_secs"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                    build_integer_row(
                        plugin,
                        "Capacity",
                        config_path(["fetch", "cache", "capacity"]),
                        (!fetch_enabled).then(|| "Fetch is disabled".to_owned()),
                    ),
                ],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "crawl",
        "Crawl",
        config_path(["crawl"]),
        None,
        vec![
            ConfigGroupView {
                title: "Crawl Range".to_owned(),
                layout: ConfigGroupLayout::Pair {
                    left_label: "Value",
                    right_label: "Limit",
                },
                rows: vec![
                    build_pair_integer_row(
                        plugin,
                        "Max pages",
                        config_path(["crawl", "defaults", "max_pages"]),
                        config_path(["crawl", "limits", "max_pages"]),
                        None,
                    ),
                    build_pair_integer_row(
                        plugin,
                        "Max depth",
                        config_path(["crawl", "defaults", "max_depth"]),
                        config_path(["crawl", "limits", "max_depth"]),
                        None,
                    ),
                    build_bool_row(
                        plugin,
                        "Same host only",
                        config_path(["crawl", "defaults", "same_host_only"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Indexing".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Document cache TTL",
                        config_path(["crawl", "indexing", "document_cache_ttl_secs"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Chunk size",
                        config_path(["crawl", "indexing", "chunk_chars"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Near-duplicate distance",
                        config_path(["crawl", "indexing", "near_duplicate_hamming_distance"]),
                        None,
                    ),
                ],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "search",
        "Search",
        config_path(["search"]),
        None,
        vec![ConfigGroupView {
            title: "Search Results".to_owned(),
            layout: ConfigGroupLayout::Pair {
                left_label: "Value",
                right_label: "Max",
            },
            rows: vec![build_pair_integer_row(
                plugin,
                "Result limit",
                config_path(["search", "default_limit"]),
                config_path(["search", "max_limit"]),
                None,
            )],
        }],
    ));

    sections.push(web_form_section(
        plugin,
        "store",
        "Store",
        config_path(["store"]),
        None,
        vec![
            ConfigGroupView {
                title: "Retention".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_integer_row(
                        plugin,
                        "Max documents",
                        config_path(["store", "retention", "max_documents"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Max bytes",
                        config_path(["store", "retention", "max_bytes"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Listing".to_owned(),
                layout: ConfigGroupLayout::Pair {
                    left_label: "Value",
                    right_label: "Max",
                },
                rows: vec![build_pair_integer_row(
                    plugin,
                    "List limit",
                    config_path(["store", "listing", "default_limit"]),
                    config_path(["store", "listing", "max_limit"]),
                    None,
                )],
            },
        ],
    ));

    sections.push(web_form_section(
        plugin,
        "browser",
        "Browser",
        config_path(["browser"]),
        None,
        vec![
            ConfigGroupView {
                title: "Browser".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_bool_row(plugin, "Enabled", config_path(["browser", "enabled"]), None),
                    build_nullable_string_row(
                        plugin,
                        "Executable path",
                        config_path(["browser", "executable_path"]),
                        None,
                    ),
                ],
            },
            ConfigGroupView {
                title: "Wait".to_owned(),
                layout: ConfigGroupLayout::Standard,
                rows: vec![
                    build_bool_row(
                        plugin,
                        "Network idle",
                        config_path(["browser", "wait", "for_network_idle"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Timeout",
                        config_path(["browser", "wait", "timeout_secs"]),
                        None,
                    ),
                    build_nullable_string_row(
                        plugin,
                        "Selector",
                        config_path(["browser", "wait", "for_selector"]),
                        None,
                    ),
                    build_integer_row(
                        plugin,
                        "Extra delay",
                        config_path(["browser", "wait", "delay_ms"]),
                        None,
                    ),
                ],
            },
        ],
    ));
    sections
}

pub(crate) fn build_generic_config_sections(
    plugin: &PluginWorkbenchPlugin,
) -> Vec<ConfigSectionView> {
    let mut sections = vec![build_generic_overview_section(plugin)];
    let root_value = &plugin.draft_config;
    let root_schema = plugin.schema.as_ref();
    if let Some(object) = root_value.as_object() {
        for key in ordered_object_keys(root_schema, object) {
            let path = vec![PathSegment::Key(key.clone())];
            sections.push(build_generic_section(
                plugin,
                &path,
                title_for_config_path(plugin, &path, key.as_str()),
            ));
        }
    } else {
        sections.push(build_generic_section(
            plugin,
            &Vec::new(),
            "Config".to_owned(),
        ));
    }
    sections
}
