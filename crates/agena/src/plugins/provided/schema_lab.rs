//! `agena.schema_lab` plugin: a built-in fixture for exercising the
//! structured plugin config editor against deep, heterogeneous JSON Schema.

use agena_macros::{StaticToolSurface, ToolInputShape};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::plugin::sdk::{
    PluginStudioCommand, PluginUiAction, Result as SdkResult, ToolInvokeOutput, ToolTag,
};

pub(crate) const SCHEMA_LAB_PLUGIN_ID: &str = "agena.schema_lab";

pub(crate) struct SchemaLabPlugin;

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInputShape, Default)]
#[tool_input(trim("section"), non_empty_if_present("section"))]
#[serde(default, deny_unknown_fields)]
struct SchemaLabInspectArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    section: Option<String>,
    #[serde(default)]
    include_defaults: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInputShape, Default)]
#[tool_input(trim("label"), non_empty_if_present("label"))]
#[serde(default, deny_unknown_fields)]
struct SchemaLabEchoArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<JsonValue>,
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "schema_lab",
    description = "No-op inspection tool for the built-in schema lab fixture plugin.",
    summary = "Inspect the schema lab fixture without mutating external state.",
    help = "Use action `inspect` to summarize one config section or `echo` to round-trip a payload into the tool result. The tool is intentionally inert and exists only to populate the Tools tab for the schema lab demo plugin.",
    handler_receiver = SchemaLabPlugin,
    ui_display = brief,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SchemaLabToolInput {
    #[tool(
        exec = "inspect",
        handle = SchemaLabPlugin::inspect,
        handle_by_value = true
    )]
    Inspect {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: SchemaLabInspectArgs,
    },
    #[tool(exec = "echo", handle = SchemaLabPlugin::echo, handle_by_value = true)]
    Echo {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: SchemaLabEchoArgs,
    },
}

impl SchemaLabPlugin {
    pub(crate) fn new() -> Self {
        Self
    }

    async fn inspect(&self, args: SchemaLabInspectArgs) -> SdkResult<ToolInvokeOutput> {
        let SchemaLabInspectArgs {
            section,
            include_defaults,
        } = args;
        let target = section.unwrap_or_else(|| "all sections".to_owned());
        Ok(ToolInvokeOutput::text(format!(
            "agena.schema_lab is a no-op fixture plugin.\nRequested section: {target}\nInclude defaults: {include_defaults}"
        ))
        .with_title("Schema Lab")
        .with_payload(json!({
            "section": target,
            "include_defaults": include_defaults,
            "mode": "inspect"
        })))
    }

    async fn echo(&self, args: SchemaLabEchoArgs) -> SdkResult<ToolInvokeOutput> {
        let SchemaLabEchoArgs { label, payload } = args;
        let label = label.unwrap_or_else(|| "schema-lab".to_owned());
        Ok(ToolInvokeOutput::text(format!(
            "Schema lab echo for `{label}` completed without side effects."
        ))
        .with_title("Schema Lab")
        .with_payload(json!({
            "label": label,
            "payload": payload,
            "mode": "echo"
        })))
    }
}

#[crate::plugin::sdk::plugin(
    id = SCHEMA_LAB_PLUGIN_ID,
    version = env!("CARGO_PKG_VERSION"),
    description = "Deep built-in JSON Schema fixture used to demo and test the structured plugin config editor.",
    config_schema = schema_lab_config_schema(),
    display = brief,
    commands = schema_lab_commands()
)]
impl SchemaLabPlugin {
    #[tool]
    async fn tool_invoke(&self, input: SchemaLabToolInput) -> SdkResult<ToolInvokeOutput> {
        input.dispatch_tool_invoke(self).await
    }
}

fn schema_lab_commands() -> Vec<PluginStudioCommand> {
    vec![
        PluginStudioCommand {
            id: "schema_lab.open_fixture".to_owned(),
            title: "Schema Lab: Open Fixture".to_owned(),
            description:
                "Placeholder command used to populate the Commands tab for the schema lab plugin."
                    .to_owned(),
            category: "Demo".to_owned(),
            slash: Some("/schema-lab".to_owned()),
            aliases: vec!["fixture".to_owned(), "schema-demo".to_owned()],
            usage: Some("No-op command palette entry for config editor demos.".to_owned()),
            location: "command_palette".to_owned(),
            action: PluginUiAction::None,
        },
        PluginStudioCommand {
            id: "schema_lab.show_defaults".to_owned(),
            title: "Schema Lab: Show Defaults".to_owned(),
            description: "Placeholder command describing the full default config fixture."
                .to_owned(),
            category: "Demo".to_owned(),
            slash: None,
            aliases: vec!["schema-defaults".to_owned()],
            usage: Some("No-op command used for Commands tab rendering.".to_owned()),
            location: "command_palette".to_owned(),
            action: PluginUiAction::None,
        },
        PluginStudioCommand {
            id: "schema_lab.run_probe".to_owned(),
            title: "Schema Lab: Run Probe".to_owned(),
            description: "Placeholder command for testing command metadata rendering.".to_owned(),
            category: "Demo".to_owned(),
            slash: None,
            aliases: vec!["schema-probe".to_owned()],
            usage: Some("No-op command. Exists only to exercise command listings.".to_owned()),
            location: "command_palette".to_owned(),
            action: PluginUiAction::None,
        },
    ]
}

fn schema_lab_config_schema() -> JsonValue {
    serde_json::from_str(
        r##"
{
  "title": "Schema Lab Config",
  "description": "Large built-in fixture schema used to exercise the structured plugin config editor across deep nesting, arrays, unions, maps, tuples, refs, and validation-heavy fields.",
  "type": "object",
  "additionalProperties": false,
  "default": {
    "identity": {
      "fixture_kind": "schema_lab",
      "display_name": "Schema Lab",
      "profile_slug": "schema-lab",
      "owner_email": "ops@example.com",
      "documentation_url": "https://docs.example.com/schema-lab",
      "hostname": "schema-lab.local",
      "instance_uuid": "123e4567-e89b-12d3-a456-426614174000",
      "notes": "This built-in fixture exists to stress the structured config editor with deep, mixed JSON Schema constructs.",
      "tags": ["primary", "docs", "demo"]
    },
    "transport": {
      "kind": "http",
      "endpoint": "https://api.example.com/v1",
      "headers": {
        "x-demo-mode": "enabled",
        "x-owner": "schema-lab"
      },
      "timeout_ms": 2500
    },
    "credentials": {
      "mode": "api_key",
      "reference": "secret://schema-lab/default",
      "header_name": "X-API-Key"
    },
    "limits": {
      "concurrency": 4,
      "burst_rate": 1.5,
      "suspended_until": null,
      "retry": {
        "enabled": true,
        "max_attempts": 3,
        "backoff": {
          "kind": "exponential",
          "base_ms": 200,
          "multiplier": 2.0
        }
      },
      "quotas": {
        "soft": 100,
        "hard": 500,
        "alert_threshold": 0.8
      }
    },
    "pipelines": [
      {
        "name": "crawl-and-index",
        "enabled": true,
        "schedule": ["0", "*/30", "*", "*", "*"],
        "steps": [
          {
            "kind": "fetch",
            "url": "https://docs.example.com",
            "method": "GET",
            "headers": {
              "x-trace-id": "fixture"
            }
          },
          {
            "kind": "transform",
            "script": "return input;",
            "timeout_secs": 20
          },
          {
            "kind": "publish",
            "channel": "search-index",
            "batch_size": 100
          }
        ]
      }
    ],
    "maps": {
      "headers": {
        "x-demo": "true",
        "x-region": "apac"
      },
      "feature_flags": {
        "beta_ui": true,
        "streaming": false
      },
      "named_limits": {
        "fast_path": 8,
        "slow_path": 2
      },
      "metadata": {
        "owner": "schema-lab",
        "priority": 3,
        "ephemeral": false,
        "comment": null
      },
      "region_policies": {
        "apac": {
          "priority": 1,
          "labels": ["edge", "gpu"]
        },
        "eu-west": {
          "priority": 2,
          "labels": ["core"]
        }
      }
    },
    "collection_mesh": {
      "list_routes": [
        {
          "name": "alpha",
          "buckets": {
            "edge": {
              "enabled": true,
              "labels": ["core", "gpu"],
              "weights": [2, 4, 8]
            },
            "cold": {
              "enabled": false,
              "labels": ["cold"],
              "weights": [1]
            }
          }
        },
        {
          "name": "beta",
          "buckets": {
            "archive": {
              "enabled": true,
              "labels": ["core"],
              "weights": [3, 6]
            }
          }
        }
      ],
      "bucket_steps": {
        "priority": [
          { "kind": "delay", "ms": 120 },
          { "kind": "label", "value": "fast-lane" }
        ],
        "fallback": [
          { "kind": "toggle", "enabled": true },
          { "kind": "script", "code": "return input;" }
        ]
      },
      "matrix_rows": [
        [
          { "key": "cpu", "value": 2 },
          { "key": "mem", "value": 8 }
        ],
        [
          { "key": "cpu", "value": 4 },
          { "key": "mem", "value": 16 }
        ]
      ]
    },
    "tuples": {
      "command": ["npx", "@agena/mcp-demo", ["--port", "7788"]],
      "command_with_tail": ["node", "worker.mjs", "--watch", "--json"],
      "coordinates": [12.5, 48.1, "warehouse-a"],
      "fallback_pair": [
        null,
        {
          "kind": "local",
          "path": "/var/lib/schema-lab/mirror",
          "watch": true
        }
      ]
    },
    "experiments": {
      "generic_payload": {
        "mode": "demo",
        "enabled": true
      },
      "notification_target": {
        "kind": "email",
        "address": "alerts@example.com"
      },
      "retention_policy": {
        "enabled": true,
        "days": 14,
        "archive_tier": "warm"
      },
      "rollout": {
        "strategy": "gradual",
        "percentage": 25,
        "window_secs": 3600
      },
      "option_matrix": [
        "docs",
        {
          "name": "safe",
          "enabled": true
        },
        3
      ],
      "enabled_regions": ["apac", "eu-west"],
      "audit": {
        "enabled": true,
        "webhook": "https://hooks.example.com/schema-lab",
        "secret_ref": "secret://schema-lab/audit"
      }
    },
    "deep_nesting": {
      "level1": {
        "level2": {
          "level3": {
            "level4": {
              "level5": {
                "level6": {
                  "level7": {
                    "terminal_message": "deep value",
                    "terminal_codes": [1, 2, 3],
                    "terminal_map": {
                      "mode": "deep",
                      "active": true
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  },
  "$defs": {
    "httpTransport": {
      "title": "HTTP Transport",
      "type": "object",
      "additionalProperties": false,
      "required": ["kind", "endpoint", "timeout_ms"],
      "properties": {
        "kind": { "const": "http", "title": "Kind" },
        "endpoint": {
          "type": "string",
          "format": "uri",
          "title": "Endpoint",
          "examples": ["https://api.example.com/v1"]
        },
        "headers": {
          "type": "object",
          "title": "Headers",
          "propertyNames": { "pattern": "^x-[a-z0-9-]+$" },
          "patternProperties": {
            "^x-[a-z0-9-]+$": { "type": "string", "minLength": 1 }
          },
          "additionalProperties": false
        },
        "timeout_ms": {
          "type": "integer",
          "minimum": 100,
          "maximum": 120000,
          "title": "Timeout"
        }
      }
    },
    "stdioTransport": {
      "title": "Stdio Transport",
      "type": "object",
      "additionalProperties": false,
      "required": ["kind", "command", "args"],
      "properties": {
        "kind": { "const": "stdio", "title": "Kind" },
        "command": { "type": "string", "minLength": 1, "title": "Command" },
        "args": {
          "type": "array",
          "title": "Args",
          "items": { "type": "string" }
        },
        "env": {
          "type": "object",
          "title": "Env",
          "additionalProperties": { "type": "string" }
        }
      }
    },
    "localTransport": {
      "title": "Local Transport",
      "type": "object",
      "additionalProperties": false,
      "required": ["kind", "path", "watch"],
      "properties": {
        "kind": { "const": "local", "title": "Kind" },
        "path": { "type": "string", "title": "Path" },
        "watch": { "type": "boolean", "title": "Watch" }
      }
    },
    "retryBackoff": {
      "title": "Retry Backoff",
      "type": "object",
      "additionalProperties": false,
      "required": ["kind", "base_ms", "multiplier"],
      "properties": {
        "kind": {
          "type": "string",
          "enum": ["fixed", "exponential"],
          "title": "Kind"
        },
        "base_ms": {
          "type": "integer",
          "minimum": 1,
          "title": "Base Delay"
        },
        "multiplier": {
          "type": "number",
          "minimum": 1.0,
          "maximum": 10.0,
          "multipleOf": 0.5,
          "title": "Multiplier"
        }
      }
    },
    "pipelineStep": {
      "title": "Pipeline Step",
      "oneOf": [
        {
          "title": "Fetch Step",
          "type": "object",
          "additionalProperties": false,
          "required": ["kind", "url", "method"],
          "properties": {
            "kind": { "const": "fetch", "title": "Kind" },
            "url": { "type": "string", "format": "uri", "title": "URL" },
            "method": {
              "type": "string",
              "enum": ["GET", "POST"],
              "title": "Method"
            },
            "headers": {
              "type": "object",
              "title": "Headers",
              "additionalProperties": { "type": "string" }
            }
          }
        },
        {
          "title": "Transform Step",
          "type": "object",
          "additionalProperties": false,
          "required": ["kind", "script", "timeout_secs"],
          "properties": {
            "kind": { "const": "transform", "title": "Kind" },
            "script": { "type": "string", "minLength": 1, "title": "Script" },
            "timeout_secs": {
              "type": "integer",
              "minimum": 1,
              "title": "Timeout"
            }
          }
        },
        {
          "title": "Publish Step",
          "type": "object",
          "additionalProperties": false,
          "required": ["kind", "channel", "batch_size"],
          "properties": {
            "kind": { "const": "publish", "title": "Kind" },
            "channel": { "type": "string", "minLength": 1, "title": "Channel" },
            "batch_size": {
              "type": "integer",
              "minimum": 1,
              "maximum": 1000,
              "title": "Batch Size"
            }
          }
        }
      ]
    },
    "pipeline": {
      "title": "Pipeline",
      "type": "object",
      "additionalProperties": false,
      "required": ["name", "enabled", "schedule", "steps"],
      "properties": {
        "name": { "type": "string", "minLength": 1, "title": "Name" },
        "enabled": { "type": "boolean", "title": "Enabled" },
        "schedule": {
          "type": "array",
          "title": "Schedule Tuple",
          "prefixItems": [
            { "type": "string", "title": "Minute" },
            { "type": "string", "title": "Hour" },
            { "type": "string", "title": "Day" },
            { "type": "string", "title": "Month" },
            { "type": "string", "title": "Weekday" }
          ],
          "items": false,
          "minItems": 5,
          "maxItems": 5
        },
        "steps": {
          "type": "array",
          "title": "Steps",
          "minItems": 1,
          "items": { "$ref": "#/$defs/pipelineStep" }
        }
      }
    },
    "notificationTarget": {
      "title": "Notification Target",
      "oneOf": [
        {
          "title": "Email Target",
          "type": "object",
          "additionalProperties": false,
          "required": ["kind", "address"],
          "properties": {
            "kind": { "const": "email", "title": "Kind" },
            "address": { "type": "string", "format": "email", "title": "Address" }
          }
        },
        {
          "title": "Webhook Target",
          "type": "object",
          "additionalProperties": false,
          "required": ["kind", "url"],
          "properties": {
            "kind": { "const": "webhook", "title": "Kind" },
            "url": { "type": "string", "format": "uri", "title": "URL" }
          }
        }
      ]
    }
  },
  "properties": {
    "identity": {
      "title": "Identity",
      "description": "Top-level string, enum, const, formatted string, deprecated, and array fields.",
      "type": "object",
      "additionalProperties": false,
      "required": ["fixture_kind", "display_name", "profile_slug", "owner_email", "documentation_url", "hostname", "instance_uuid", "notes", "tags"],
      "properties": {
        "fixture_kind": { "title": "Fixture Kind", "const": "schema_lab", "readOnly": true },
        "display_name": { "title": "Display Name", "type": "string", "minLength": 3, "maxLength": 40 },
        "profile_slug": {
          "title": "Profile Slug",
          "type": "string",
          "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"
        },
        "owner_email": { "title": "Owner Email", "type": "string", "format": "email" },
        "documentation_url": { "title": "Documentation URL", "type": "string", "format": "uri" },
        "hostname": { "title": "Hostname", "type": "string", "format": "hostname" },
        "instance_uuid": { "title": "Instance UUID", "type": "string", "format": "uuid" },
        "notes": { "title": "Notes", "type": "string", "minLength": 10 },
        "tags": {
          "title": "Tags",
          "type": "array",
          "items": { "type": "string", "minLength": 1 },
          "minItems": 1,
          "uniqueItems": true,
          "contains": { "const": "primary" }
        },
        "legacy_profile_id": {
          "title": "Legacy Profile ID",
          "type": "integer",
          "minimum": 1,
          "deprecated": true
        }
      }
    },
    "transport": {
      "title": "Transport",
      "description": "One-of transport selection using discriminator-style const fields.",
      "oneOf": [
        { "$ref": "#/$defs/httpTransport" },
        { "$ref": "#/$defs/stdioTransport" },
        { "$ref": "#/$defs/localTransport" }
      ]
    },
    "credentials": {
      "title": "Credentials",
      "description": "Any-of credential shapes with different required fields.",
      "anyOf": [
        {
          "title": "API Key",
          "type": "object",
          "additionalProperties": false,
          "required": ["mode", "reference", "header_name"],
          "properties": {
            "mode": { "const": "api_key", "title": "Mode" },
            "reference": { "type": "string", "minLength": 1, "title": "Reference" },
            "header_name": { "type": "string", "minLength": 1, "title": "Header Name" }
          }
        },
        {
          "title": "Bearer Token",
          "type": "object",
          "additionalProperties": false,
          "required": ["mode", "reference"],
          "properties": {
            "mode": { "const": "bearer", "title": "Mode" },
            "reference": { "type": "string", "minLength": 1, "title": "Reference" }
          }
        },
        {
          "title": "Anonymous",
          "type": "object",
          "additionalProperties": false,
          "required": ["mode", "audit_reason"],
          "properties": {
            "mode": { "const": "anonymous", "title": "Mode" },
            "audit_reason": { "type": "string", "minLength": 3, "title": "Audit Reason" }
          }
        }
      ]
    },
    "limits": {
      "title": "Limits",
      "description": "Numeric constraints, null values, and all-of merged retry settings.",
      "type": "object",
      "additionalProperties": false,
      "required": ["concurrency", "burst_rate", "suspended_until", "retry", "quotas"],
      "properties": {
        "concurrency": { "title": "Concurrency", "type": "integer", "minimum": 1, "maximum": 64 },
        "burst_rate": { "title": "Burst Rate", "type": "number", "minimum": 0.5, "maximum": 10.0, "multipleOf": 0.5 },
        "suspended_until": { "title": "Suspended Until", "type": "null" },
        "retry": {
          "title": "Retry Policy",
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "enabled": true,
            "max_attempts": true,
            "backoff": true
          },
          "allOf": [
            {
              "type": "object",
              "required": ["enabled", "max_attempts"],
              "properties": {
                "enabled": { "type": "boolean", "title": "Enabled" },
                "max_attempts": { "type": "integer", "minimum": 1, "maximum": 10, "title": "Max Attempts" }
              }
            },
            {
              "type": "object",
              "required": ["backoff"],
              "properties": {
                "backoff": { "$ref": "#/$defs/retryBackoff" }
              }
            }
          ]
        },
        "quotas": {
          "title": "Quotas",
          "type": "object",
          "additionalProperties": false,
          "required": ["soft", "hard", "alert_threshold"],
          "properties": {
            "soft": { "type": "integer", "minimum": 1, "title": "Soft Limit" },
            "hard": { "type": "integer", "minimum": 1, "title": "Hard Limit" },
            "alert_threshold": { "type": "number", "minimum": 0.1, "maximum": 1.0, "multipleOf": 0.05, "title": "Alert Threshold" }
          }
        }
      }
    },
    "pipelines": {
      "title": "Pipelines",
      "description": "Array of nested objects, tuple arrays, and one-of step shapes.",
      "type": "array",
      "minItems": 1,
      "items": { "$ref": "#/$defs/pipeline" }
    },
    "maps": {
      "title": "Maps",
      "description": "Pattern-key maps, typed additionalProperties, and heterogeneous metadata values.",
      "type": "object",
      "additionalProperties": false,
      "required": ["headers", "feature_flags", "named_limits", "metadata", "region_policies"],
      "properties": {
        "headers": {
          "title": "Headers",
          "type": "object",
          "propertyNames": { "pattern": "^x-[a-z0-9-]+$" },
          "patternProperties": {
            "^x-[a-z0-9-]+$": { "type": "string", "minLength": 1 }
          },
          "additionalProperties": false
        },
        "feature_flags": {
          "title": "Feature Flags",
          "type": "object",
          "additionalProperties": { "type": "boolean" }
        },
        "named_limits": {
          "title": "Named Limits",
          "type": "object",
          "additionalProperties": { "type": "integer", "minimum": 1 }
        },
        "metadata": {
          "title": "Metadata",
          "type": "object",
          "additionalProperties": {
            "oneOf": [
              { "type": "string" },
              { "type": "number" },
              { "type": "boolean" },
              { "type": "null" }
            ]
          }
        },
        "region_policies": {
          "title": "Region Policies",
          "type": "object",
          "additionalProperties": {
            "type": "object",
            "additionalProperties": false,
            "required": ["priority", "labels"],
            "properties": {
              "priority": { "type": "integer", "minimum": 1, "title": "Priority" },
              "labels": {
                "title": "Labels",
                "type": "array",
                "items": {
                  "type": "string",
                  "enum": ["edge", "core", "cold", "gpu"]
                },
                "uniqueItems": true,
                "minItems": 1
              }
            }
          }
        }
      }
    },
    "collection_mesh": {
      "title": "Collection Mesh",
      "description": "Cross-nested collections covering list-of-map-of-object-of-list, map-of-array-of-union, and array-of-array-of-object layouts.",
      "type": "object",
      "additionalProperties": false,
      "required": ["list_routes", "bucket_steps", "matrix_rows"],
      "properties": {
        "list_routes": {
          "title": "List Routes",
          "type": "array",
          "minItems": 1,
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "buckets"],
            "properties": {
              "name": {
                "type": "string",
                "minLength": 1,
                "title": "Name"
              },
              "buckets": {
                "title": "Buckets",
                "type": "object",
                "additionalProperties": {
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["enabled", "labels", "weights"],
                  "properties": {
                    "enabled": { "type": "boolean", "title": "Enabled" },
                    "labels": {
                      "title": "Labels",
                      "type": "array",
                      "items": {
                        "type": "string",
                        "enum": ["core", "gpu", "cold", "edge", "archive"]
                      },
                      "uniqueItems": true,
                      "minItems": 1
                    },
                    "weights": {
                      "title": "Weights",
                      "type": "array",
                      "items": {
                        "type": "integer",
                        "minimum": 1
                      },
                      "minItems": 1
                    }
                  }
                }
              }
            }
          }
        },
        "bucket_steps": {
          "title": "Bucket Steps",
          "type": "object",
          "additionalProperties": {
            "type": "array",
            "minItems": 1,
            "items": {
              "oneOf": [
                {
                  "title": "Delay Step",
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["kind", "ms"],
                  "properties": {
                    "kind": { "const": "delay", "title": "Kind" },
                    "ms": { "type": "integer", "minimum": 1, "title": "Delay" }
                  }
                },
                {
                  "title": "Label Step",
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["kind", "value"],
                  "properties": {
                    "kind": { "const": "label", "title": "Kind" },
                    "value": { "type": "string", "minLength": 1, "title": "Value" }
                  }
                },
                {
                  "title": "Toggle Step",
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["kind", "enabled"],
                  "properties": {
                    "kind": { "const": "toggle", "title": "Kind" },
                    "enabled": { "type": "boolean", "title": "Enabled" }
                  }
                },
                {
                  "title": "Script Step",
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["kind", "code"],
                  "properties": {
                    "kind": { "const": "script", "title": "Kind" },
                    "code": { "type": "string", "minLength": 1, "title": "Code" }
                  }
                }
              ]
            }
          }
        },
        "matrix_rows": {
          "title": "Matrix Rows",
          "type": "array",
          "minItems": 1,
          "items": {
            "type": "array",
            "minItems": 1,
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["key", "value"],
              "properties": {
                "key": { "type": "string", "minLength": 1, "title": "Key" },
                "value": { "type": "integer", "minimum": 0, "title": "Value" }
              }
            }
          }
        }
      }
    },
    "tuples": {
      "title": "Tuples",
      "description": "Prefix-item tuples containing strings, arrays, numbers, nulls, and nested refs.",
      "type": "object",
      "additionalProperties": false,
      "required": ["command", "command_with_tail", "coordinates", "fallback_pair"],
      "properties": {
        "command": {
          "title": "Command Tuple",
          "type": "array",
          "prefixItems": [
            { "type": "string", "title": "Executable" },
            { "type": "string", "title": "Package" },
            { "type": "array", "title": "Args", "items": { "type": "string" } }
          ],
          "items": false,
          "minItems": 3,
          "maxItems": 3
        },
        "command_with_tail": {
          "title": "Command With Tail",
          "type": "array",
          "prefixItems": [
            { "type": "string", "title": "Runtime" },
            { "type": "string", "title": "Entry" }
          ],
          "items": { "type": "string", "title": "Extra Arg" },
          "minItems": 2
        },
        "coordinates": {
          "title": "Coordinates",
          "type": "array",
          "prefixItems": [
            { "type": "number", "title": "Latitude" },
            { "type": "number", "title": "Longitude" },
            { "type": "string", "title": "Site" }
          ],
          "items": false,
          "minItems": 3,
          "maxItems": 3
        },
        "fallback_pair": {
          "title": "Fallback Pair",
          "type": "array",
          "prefixItems": [
            { "type": "null", "title": "Primary" },
            {
              "oneOf": [
                { "$ref": "#/$defs/httpTransport" },
                { "$ref": "#/$defs/localTransport" }
              ],
              "title": "Secondary"
            }
          ],
          "items": false,
          "minItems": 2,
          "maxItems": 2
        }
      }
    },
    "experiments": {
      "title": "Experiments",
      "description": "Union-heavy section covering generic payloads, one-of targets, all-of policies, conditional requirements, and mixed arrays.",
      "type": "object",
      "additionalProperties": false,
      "required": ["generic_payload", "notification_target", "retention_policy", "rollout", "option_matrix", "enabled_regions", "audit"],
      "properties": {
        "generic_payload": {
          "title": "Generic Payload",
          "type": ["string", "number", "boolean", "object", "array", "null"]
        },
        "notification_target": { "$ref": "#/$defs/notificationTarget" },
        "retention_policy": {
          "title": "Retention Policy",
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "enabled": true,
            "days": true,
            "archive_tier": true
          },
          "allOf": [
            {
              "type": "object",
              "required": ["enabled", "days"],
              "properties": {
                "enabled": { "type": "boolean", "title": "Enabled" },
                "days": { "type": "integer", "minimum": 1, "maximum": 365, "title": "Days" }
              }
            },
            {
              "type": "object",
              "required": ["archive_tier"],
              "properties": {
                "archive_tier": {
                  "type": "string",
                  "enum": ["hot", "warm", "cold"],
                  "title": "Archive Tier"
                }
              }
            }
          ]
        },
        "rollout": {
          "title": "Rollout",
          "type": "object",
          "additionalProperties": false,
          "required": ["strategy"],
          "properties": {
            "strategy": { "type": "string", "enum": ["gradual", "instant"], "title": "Strategy" },
            "percentage": { "type": "integer", "minimum": 0, "maximum": 100, "title": "Percentage" },
            "window_secs": { "type": "integer", "minimum": 0, "title": "Window" }
          },
          "if": { "properties": { "strategy": { "const": "gradual" } } },
          "then": { "required": ["percentage", "window_secs"] }
        },
        "option_matrix": {
          "title": "Option Matrix",
          "type": "array",
          "items": {
            "oneOf": [
              { "type": "string" },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "enabled"],
                "properties": {
                  "name": { "type": "string", "title": "Name" },
                  "enabled": { "type": "boolean", "title": "Enabled" }
                }
              },
              { "type": "integer" }
            ]
          }
        },
        "enabled_regions": {
          "title": "Enabled Regions",
          "type": "array",
          "items": {
            "type": "string",
            "enum": ["us-east", "eu-west", "apac", "internal"]
          },
          "uniqueItems": true,
          "minItems": 1
        },
        "audit": {
          "title": "Audit",
          "type": "object",
          "additionalProperties": false,
          "required": ["enabled"],
          "properties": {
            "enabled": { "type": "boolean", "title": "Enabled" },
            "webhook": { "type": "string", "format": "uri", "title": "Webhook" },
            "secret_ref": { "type": "string", "minLength": 1, "title": "Secret Reference" }
          },
          "dependentRequired": {
            "webhook": ["secret_ref"]
          },
          "dependentSchemas": {
            "enabled": {
              "if": {
                "properties": {
                  "enabled": { "const": true }
                }
              },
              "then": {
                "required": ["webhook", "secret_ref"]
              }
            }
          }
        }
      }
    },
    "deep_nesting": {
      "title": "Deep Nesting",
      "description": "Artificially deep nested objects used to test long navigation paths and drill-down behavior.",
      "type": "object",
      "additionalProperties": false,
      "required": ["level1"],
      "properties": {
        "level1": {
          "title": "Level 1",
          "type": "object",
          "additionalProperties": false,
          "required": ["level2"],
          "properties": {
            "level2": {
              "title": "Level 2",
              "type": "object",
              "additionalProperties": false,
              "required": ["level3"],
              "properties": {
                "level3": {
                  "title": "Level 3",
                  "type": "object",
                  "additionalProperties": false,
                  "required": ["level4"],
                  "properties": {
                    "level4": {
                      "title": "Level 4",
                      "type": "object",
                      "additionalProperties": false,
                      "required": ["level5"],
                      "properties": {
                        "level5": {
                          "title": "Level 5",
                          "type": "object",
                          "additionalProperties": false,
                          "required": ["level6"],
                          "properties": {
                            "level6": {
                              "title": "Level 6",
                              "type": "object",
                              "additionalProperties": false,
                              "required": ["level7"],
                              "properties": {
                                "level7": {
                                  "title": "Level 7",
                                  "type": "object",
                                  "additionalProperties": false,
                                  "required": ["terminal_message", "terminal_codes", "terminal_map"],
                                  "properties": {
                                    "terminal_message": { "type": "string", "title": "Terminal Message" },
                                    "terminal_codes": {
                                      "type": "array",
                                      "title": "Terminal Codes",
                                      "items": { "type": "integer" }
                                    },
                                    "terminal_map": {
                                      "type": "object",
                                      "title": "Terminal Map",
                                      "additionalProperties": false,
                                      "required": ["mode", "active"],
                                      "properties": {
                                        "mode": { "type": "string", "title": "Mode" },
                                        "active": { "type": "boolean", "title": "Active" }
                                      }
                                    }
                                  }
                                }
                              }
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
"##,
    )
    .expect("schema lab schema should parse")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::sdk::{Plugin, ToolInvokeInput};
    use serde_json::json;

    #[test]
    fn schema_lab_manifest_exposes_complex_config_schema_and_commands() {
        let manifest = SchemaLabPlugin::new().manifest();
        assert_eq!(manifest.name, SCHEMA_LAB_PLUGIN_ID);
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.commands.len(), 3);
        let schema = manifest.config_schema.expect("schema lab config schema");
        assert_eq!(
            schema.get("type").and_then(JsonValue::as_str),
            Some("object")
        );
        let properties = schema
            .get("properties")
            .and_then(JsonValue::as_object)
            .expect("schema properties");
        for key in [
            "identity",
            "transport",
            "credentials",
            "limits",
            "pipelines",
            "maps",
            "collection_mesh",
            "tuples",
            "experiments",
            "deep_nesting",
        ] {
            assert!(properties.contains_key(key), "missing root section {key}");
        }
        let identity = properties
            .get("identity")
            .and_then(|value| value.get("properties"))
            .and_then(JsonValue::as_object)
            .expect("identity properties");
        assert!(identity.contains_key("profile_slug"));

        let maps = properties
            .get("maps")
            .and_then(|value| value.get("properties"))
            .and_then(JsonValue::as_object)
            .expect("maps properties");
        assert!(maps.contains_key("region_policies"));

        let collection_mesh = properties
            .get("collection_mesh")
            .and_then(|value| value.get("properties"))
            .and_then(JsonValue::as_object)
            .expect("collection_mesh properties");
        assert!(collection_mesh.contains_key("list_routes"));
        assert!(collection_mesh.contains_key("bucket_steps"));
        assert!(collection_mesh.contains_key("matrix_rows"));

        let experiments = properties
            .get("experiments")
            .and_then(|value| value.get("properties"))
            .and_then(JsonValue::as_object)
            .expect("experiments properties");
        assert!(experiments.contains_key("enabled_regions"));
        assert!(experiments.contains_key("audit"));
    }

    #[test]
    fn schema_lab_tool_resolve_tool_preserves_declared_actions() {
        let (inspect_action, inspect_args) = SchemaLabToolInput::resolve_tool(
            "schema_lab",
            json!({
                "action": "inspect",
                "section": "identity",
                "include_defaults": true
            }),
        )
        .expect("inspect action should resolve");
        assert_eq!(inspect_action, "inspect");
        assert_eq!(
            inspect_args,
            json!({
                "section": "identity",
                "include_defaults": true
            })
        );

        let (echo_action, echo_args) = SchemaLabToolInput::resolve_tool(
            "schema_lab",
            json!({
                "action": "echo",
                "label": "fixture",
                "payload": { "ok": true }
            }),
        )
        .expect("echo action should resolve");
        assert_eq!(echo_action, "echo");
        assert_eq!(
            echo_args,
            json!({
                "label": "fixture",
                "payload": { "ok": true }
            })
        );
    }

    #[test]
    fn schema_lab_tool_inputs_trim_optional_fields_through_flattened_shapes() {
        let parsed = SchemaLabToolInput::parse_input(json!({
            "action": "inspect",
            "section": "  identity.maps  ",
            "include_defaults": true
        }))
        .expect("schema_lab inspect should trim section during parse");
        match parsed {
            SchemaLabToolInput::Inspect { args } => {
                assert_eq!(args.section.as_deref(), Some("identity.maps"));
                assert!(args.include_defaults);
            }
            other => panic!("expected inspect variant, got {other:?}"),
        }

        let parsed = SchemaLabToolInput::parse_input(json!({
            "action": "echo",
            "label": "  fixture-demo  "
        }))
        .expect("schema_lab echo should trim label during parse");
        match parsed {
            SchemaLabToolInput::Echo { args } => {
                assert_eq!(args.label.as_deref(), Some("fixture-demo"));
            }
            other => panic!("expected echo variant, got {other:?}"),
        }

        let err = SchemaLabToolInput::parse_input(json!({
            "action": "inspect",
            "section": "   "
        }))
        .expect_err("schema_lab inspect should reject blank section when provided");
        assert!(
            err.to_string()
                .contains("field `section` must not be empty")
        );
    }

    #[test]
    fn schema_lab_tool_invoke_supports_inspect_and_echo() {
        let plugin = SchemaLabPlugin::new();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");

        let inspect = runtime
            .block_on(Plugin::tool_invoke(
                &plugin,
                ToolInvokeInput {
                    tool_name: "schema_lab".to_string(),
                    session_id: 0,
                    call_id: 0,
                    workspace_root: ".".to_string(),
                    input: json!({
                        "action": "inspect",
                        "section": "maps",
                        "include_defaults": false
                    }),
                },
            ))
            .expect("inspect invocation should succeed");
        assert_eq!(inspect.title, "Schema Lab");
        assert!(inspect.output_text.contains("Requested section: maps"));
        assert_eq!(
            inspect.payload,
            Some(json!({
                "section": "maps",
                "include_defaults": false,
                "mode": "inspect"
            }))
        );

        let echo = runtime
            .block_on(Plugin::tool_invoke(
                &plugin,
                ToolInvokeInput {
                    tool_name: "schema_lab".to_string(),
                    session_id: 0,
                    call_id: 0,
                    workspace_root: ".".to_string(),
                    input: json!({
                        "action": "echo",
                        "label": "probe",
                        "payload": { "depth": 2 }
                    }),
                },
            ))
            .expect("echo invocation should succeed");
        assert_eq!(echo.title, "Schema Lab");
        assert!(echo.output_text.contains("Schema lab echo for `probe`"));
        assert_eq!(
            echo.payload,
            Some(json!({
                "label": "probe",
                "payload": { "depth": 2 },
                "mode": "echo"
            }))
        );
    }
}
