# OpenCode 接入

OpenCode 目前有两类值得单独配置的入口：

- OpenCode Go：订阅式低价模型网关，共享根路径是 `https://opencode.ai/zen/go`
- OpenCode Zen：OpenCode 内置模型网关，共享根路径是 `https://opencode.ai/zen`，没有 key 时可用 `api_key = "public"` 调免费模型

这两类入口都不需要新增 Agena credential issuer。运行时请求只需要 `provider.auth.mode = "api"`、`base_url` 和 `api_key` / `api_key_env`。以后如果要做 OpenCode 登录、keyring 自动同步或账户状态展示，可以再加 `credential` issuer；模型调用本身不依赖它。

OpenCode 的 `/models` endpoint 不按 Agena 的 adapter 维度拆分模型，所以这类 provider 推荐显式声明模型路由。免费公共入口还应该使用 `model_discovery = "configured_only"`，避免把付费模型列进可选模型。

## 路由

常见 HTTP 路由如下：

- OpenAI-compatible chat completions：`<base_url>/chat/completions`
- OpenAI Responses：`<base_url>/responses`
- Anthropic Messages：`<base_url>/messages`
- Gemini generateContent：`<base_url>/models/<model>:generateContent`

OpenCode Go 当前主要用 OpenAI-compatible 和 Anthropic Messages。OpenCode Zen 会同时包含 OpenAI-compatible、OpenAI Responses、Anthropic Messages 和 Gemini 路由。

## 准备 Key

1. 在 `https://opencode.ai/go` 开通 OpenCode Go。
2. 在 OpenCode 的 auth 页面创建并复制 API key。
3. 把 key 放到环境变量里：

```bash
export OPENCODE_API_KEY=...
```

也可以把 `api_key_env = "OPENCODE_API_KEY"` 改成 `api_key = "..."`，但本地配置里不建议直接写密钥。

免费公共模型不需要真实 key，配置里使用 `api_key = "public"`。

## OpenCode Go

把下面片段放进 `~/.agena/config.json`。默认模型使用 OpenAI-compatible 路由，MiniMax M2.7 / M2.5 使用 Anthropic Messages 路由。

```json
{
  "providers": {
    "default": "opencode-go",
    "opencode-go": {
      "defaults": {
        "adapter": "openai",
        "model": "kimi-k2.6"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://opencode.ai/zen/go",
        "api_key_env": "OPENCODE_API_KEY"
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "api_mode": "chat",
          "models_url": "https://opencode.ai/zen/go/v1/models",
          "models": {
            "minimax-m2.7": {
              "enabled": false
            },
            "minimax-m2.5": {
              "enabled": false
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "messages_url": "https://opencode.ai/zen/go/v1/messages",
          "models_url": "https://opencode.ai/zen/go/v1/models",
          "models": {
            "minimax-m2.7": {
              "enabled": true
            },
            "minimax-m2.5": {
              "enabled": true
            },
            "kimi-k2.6": {
              "enabled": false
            },
            "kimi-k2.5": {
              "enabled": false
            },
            "glm-5.1": {
              "enabled": false
            },
            "glm-5": {
              "enabled": false
            },
            "deepseek-v4-pro": {
              "enabled": false
            },
            "deepseek-v4-flash": {
              "enabled": false
            },
            "qwen3.6-plus": {
              "enabled": false
            },
            "qwen3.5-plus": {
              "enabled": false
            },
            "mimo-v2-pro": {
              "enabled": false
            },
            "mimo-v2-omni": {
              "enabled": false
            },
            "mimo-v2.5-pro": {
              "enabled": false
            },
            "mimo-v2.5": {
              "enabled": false
            },
            "hy3-preview": {
              "enabled": false
            }
          }
        }
      }
    }
  }
}
```

## 为什么这样配置

这里的 `base_url` 表示共享根路径，不再直接带协议前缀。Agena 会把 OpenAI-compatible adapter 组装成 `https://opencode.ai/zen/go/v1/chat/completions`，把 Anthropic adapter 组装成 `https://opencode.ai/zen/go/v1/messages`。OpenCode Go 的官方 `/models` endpoint 返回 Go 套餐下的模型列表，但这个列表不区分协议，所以配置里需要对 MiniMax 和非 MiniMax 模型做 adapter 级别的启用/禁用。

如果只需要 Kimi、GLM、DeepSeek、Qwen、MiMo、Hunyuan 等 OpenAI-compatible 路由模型，可以只保留 `openai` adapter，并删除整个 `anthropic` adapter 段。需要 MiniMax M2.7 / M2.5 时再启用 `anthropic` adapter。

CLI 的 `--model provider/model` 只指定 provider 和 model，不显式编码 adapter；它会使用 provider 默认 adapter。临时跑 MiniMax 时，可以把默认 adapter 改成 `anthropic`：

```bash
cargo run -p agena-cli -- \
  --set providers.default=opencode-go \
  --set providers."opencode-go".defaults.adapter=anthropic \
  --set providers."opencode-go".defaults.model=minimax-m2.7 \
  exec "hello"
```

TUI、Studio 和后端 API 可以把 `provider_id`、`adapter_id`、`model_id` 分开传递，更适合在同一个 provider 下混用两条协议路由。

## 验证

```bash
cargo run -p agena-cli -- config validate
cargo run -p agena-cli -- config resolve --format json
cargo run -p agena-cli -- provider models opencode-go
cargo run -p agena-cli -- exec --model opencode-go/kimi-k2.6 "用一句话回答：你能工作吗？"
```

如果 `provider models opencode-go` 里出现了新模型，但它被列在错误 adapter 下，刷新 `https://opencode.ai/zen/go/v1/models` 后把对应模型补到上面的 `enabled = false` 路由表即可。

## OpenCode Zen 免费模型

免费模型建议单独配置成 `opencode-free`。这里使用 `api_key = "public"`，并给每个 adapter 设置 `model_discovery = "configured_only"`，这样模型列表只会展示下面显式声明的免费模型，不会把 Zen `/models` 返回的付费模型暴露出来。

```json
{
  "providers": {
    "opencode-free": {
      "defaults": {
        "adapter": "openai",
        "model": "deepseek-v4-flash-free"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://opencode.ai/zen",
        "api_key": "public",
        "protocol_paths": {
          "gemini": "/v1"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "api_mode": "chat",
          "model_discovery": "configured_only",
          "models": {
            "deepseek-v4-flash-free": {
              "enabled": true
            },
            "kimi-k2.5-free": {
              "enabled": true
            },
            "glm-5-free": {
              "enabled": true
            },
            "glm-4.7-free": {
              "enabled": true
            },
            "mimo-v2-flash-free": {
              "enabled": true
            },
            "mimo-v2-pro-free": {
              "enabled": true
            },
            "mimo-v2-omni-free": {
              "enabled": true
            },
            "hy3-preview-free": {
              "enabled": true
            },
            "ling-2.6-flash-free": {
              "enabled": true
            },
            "ring-2.6-1t-free": {
              "enabled": true
            },
            "trinity-large-preview-free": {
              "enabled": true
            },
            "nemotron-3-super-free": {
              "enabled": true
            },
            "big-pickle": {
              "enabled": true
            },
            "grok-code": {
              "enabled": true
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "model_discovery": "configured_only",
          "models": {
            "minimax-m2.1-free": {
              "enabled": true
            },
            "minimax-m2.5-free": {
              "enabled": true
            },
            "qwen3.6-plus-free": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

运行：

```bash
cargo run -p agena-cli -- \
  --set providers.default=opencode-free \
  --set providers."opencode-free".defaults.adapter=openai \
  --set providers."opencode-free".defaults.model=deepseek-v4-flash-free \
  exec "hello"
```

如果要跑免费 MiniMax 或 Qwen，需要切到 `anthropic` adapter：

```bash
cargo run -p agena-cli -- \
  --set providers.default=opencode-free \
  --set providers."opencode-free".defaults.adapter=anthropic \
  --set providers."opencode-free".defaults.model=minimax-m2.5-free \
  exec "hello"
```

## OpenCode Zen 付费/自带模型

如果你有 `OPENCODE_API_KEY`，可以配置完整 Zen provider。建议仍然使用 `model_discovery = "configured_only"`，然后按协议把常用模型放到对应 adapter 下；这样不会在每个 adapter 下重复显示所有模型。

```json
{
  "providers": {
    "opencode-zen": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-5.5"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://opencode.ai/zen",
        "api_key_env": "OPENCODE_API_KEY",
        "protocol_paths": {
          "gemini": "/v1"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "api_mode": "auto",
          "model_discovery": "configured_only",
          "models": {
            "gpt-5.5": {
              "enabled": true
            },
            "gpt-5.3-codex": {
              "enabled": true
            },
            "kimi-k2.6": {
              "enabled": true
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "model_discovery": "configured_only",
          "models": {
            "claude-sonnet-4-6": {
              "enabled": true
            },
            "minimax-m2.7": {
              "enabled": true
            }
          }
        },
        "gemini": {
          "enabled": true,
          "auth_header": "x-goog-api-key",
          "model_discovery": "configured_only",
          "models": {
            "gemini-3.1-pro": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

Zen 需要显式把 Gemini 协议前缀改成 `/v1`。OpenCode Zen 的 Gemini 路径是 `/zen/v1/models/<model>:generateContent`，不是 Google 原生的 `/v1beta/...`；因此 `auth.protocol_paths.gemini = "/v1"` 必须单独写出来。
