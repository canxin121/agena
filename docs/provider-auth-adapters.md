# Provider / Auth / Adapter 架构

本文说明当前 provider 子系统的 canonical 结构，以及 provider、auth、adapter、model 四层各自负责什么。

## 总览

当前配置结构是：

```text
execution
├── provider
├── adapter
├── model
└── agent

provider
├── enabled
├── defaults
│   ├── adapter
│   └── model
├── auth
└── adapters
    └── <adapter>
        ├── enabled
        ├── protocol/options
        └── models
            └── <real-upstream-model-id>
                ├── enabled
                ├── native_tools
                │   ├── routes
                │   ├── hosted
                │   ├── harness
                │   └── connectors
                └── metadata / capabilities / thinking_modes / speed_modes
```

对应 JSON：

```json
{
  "execution": {
    "provider": "openai",
    "adapter": "openai",
    "model": "gpt-5",
    "agent": "build"
  },
  "providers": {
    "openai": {
      "enabled": true,
      "auth": {
        "mode": "api",
        "base_url": "https://api.openai.com",
        "api_key_env": "OPENAI_API_KEY"
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-5": {
              "enabled": true,
              "native_tools": {
                "enabled": true,
                "routes": {
                  "web_search": "provider_hosted"
                }
              }
            }
          }
        }
      }
    }
  }
}
```

关键约束：

- 全局默认 provider/adapter/model/agent 写在 `[execution]`
- `providers.<id>.defaults.adapter` 和 `providers.<id>.defaults.model` 是 provider-local 默认选择；`defaults.model` 必须是真实上游 model id
- adapter 不再有自己的默认模型字段
- model key 就是真实上游 model id，不再有 `target_model`
- provider / adapter / model 三层都支持 `enabled`
- provider-native remote tools 的路由和 hosted 默认值写在 `providers.<id>.adapters.<adapter>.models.<model>.native_tools`
- 运行时模型选择由 `provider_id`、`adapter_id`、`model_id` 三个字段共同决定，不使用三段字符串编码

当前对话 runtime 已经接通的 provider-hosted 组合是：

- OpenAI：`web_search`、`file_search`、`code_execution`
- Anthropic：`web_search`
- Gemini：`web_search`、`url_context`、`code_execution`

## 四层职责

### provider

`provider` 是 Agena 对外暴露的逻辑入口。

它负责：

- 稳定的 `provider_id`
- provider 级 `auth`
- provider-native tool 路由与 hosted defaults
- 聚合一个或多个 adapters
- 暴露 provider 默认模型
- 对外提供统一的模型命名空间

CLI、HTTP API、Studio、session 持久化、model ref 都围绕 `provider_id` 工作。

### auth

`provider.auth` 只负责认证与连接身份，不负责模型路由。

它负责：

- shared endpoint / token / OAuth / ADC / SigV4 / service key 等认证来源
- refresh 生命周期
- provider-local metadata，例如 Copilot enterprise host、ChatGPT account id
- 给同一个 provider 下所有 adapters 共享认证上下文

当一个 auth 网关同时暴露多种协议时，`auth.base_url` 表示共享根路径，`auth.protocol_paths` 显式声明每种协议挂在哪条前缀上。运行时不会再根据 URL 形状做自动推导。

`auth` 不再拆成独立 connection 对象，也不放在 adapter 上。

provider-native hosted tools 默认直接复用这一层 auth；不会再单独引入第二套 OpenAI / Anthropic / Gemini tool secret 配置。

同样因为 provider-native tool 会复用 auth provenance，Agena 的创建界面会把 auth 来源当成“建议配置”的依据：

- first-party official auth 可以在 TUI / Studio Web 里默认勾选一组保守的 hosted native tools
- 自定义 gateway / compatible auth 默认不勾选任何 native tool，但仍可手动开启 adapter 对应的 preset
- 保存后这些建议会变成显式的 `providers.<id>.adapters.<adapter>.models.<model>.native_tools.*` 配置，而不是 runtime fallback
- 用户也可以在创建阶段直接改成 `enabled = false`

### adapter

`adapter` 代表真实协议实现。

它负责：

- 选择 wire protocol
- 请求/流式协议细节
- provider-specific transport 选项
- 暴露该 adapter 下的模型集合

典型 adapter：

- `openai`
- `anthropic`
- `gemini`
- `gitlab`
- `amazon_bedrock`
- `ollama`

一个 provider 下可以有多个 adapter，但同 kind 只保留一个 canonical adapter。

### model

`providers.<id>.adapters.<adapter>.models."<model-id>"` 表示该 adapter 下一个真实上游模型的配置节点。

它负责：

- 开关控制：`enabled`
- model-scoped provider-native tools
- metadata patch
- capability patch
- thinking mode patch
- speed mode patch

这里的 key 就是真实上游 model id，例如：

- `"gpt-5"`
- `"claude-sonnet-4"`
- `"google/gemini-2.5-flash"`
- `"anthropic.claude-3-7-sonnet-20250219-v1:0"`

不再有 `target_model`。如果你写了 `models."gpt-5"`，那它路由到的就是上游的 `gpt-5`。

## 命名与路由

运行时选择模型时始终拆成三个字段：

- 全局默认字段：`[execution] provider = "openai"`, `adapter = "openai"`, `model = "gpt-5"`
- provider-local 默认选择：`defaults.adapter = "openai"`, `defaults.model = "gpt-5"`
- 真实包含 `/` 的模型名保留在 `model`/`model_id` 字段里，例如 `model = "google/gemini-2.5-flash"`

内部不再把 visible model id 编码成 `"<adapter>/<model>"` 或 `"<provider>/<adapter>/<model>"` 这样的特殊字符串；`model_id` 里的 `/` 只是模型名本身的一部分。

## enabled 语义

三层都支持 `enabled`：

```json
{
  "providers": {
    "shared": {
      "enabled": true,
      "defaults": {
        "adapter": "openai",
        "model": "gpt-4.1-mini"
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-4.1-mini": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

行为：

- provider disabled：整个 provider 不注册
- adapter disabled：该 adapter 不可选，也不会暴露其模型
- model disabled：该 adapter 下的具体 model 不可选

这三个开关都用于快速下线 provider、adapter 或单个模型，而不需要删配置。

默认值：

- provider：默认 `enabled = true`
- adapter：默认 `enabled = false`
- model：默认 `enabled = true`

因此只要你希望某个 adapter 真正对外提供模型，建议显式写上 `enabled = true`。

## auth 模式

`provider.auth.mode` 可选值：

```text
none
api
credential
bedrock_sigv4
google_adc
sap_ai_core
```

### `none`

用于本地无认证 provider，例如 `ollama`。

### `api`

用于显式 endpoint + API key：

```json
{
  "providers": {
    "openai": {
      "auth": {
        "mode": "api",
        "base_url": "https://api.openai.com",
        "api_key_env": "OPENAI_API_KEY"
      }
    }
  }
}
```

字段：

- `base_url`
- `protocol_paths`
- `api_key`
- `api_key_env`

`protocol_paths` 是 auth 级的协议前缀表，默认值是：

- `openai = "/v1"`
- `anthropic = "/v1"`
- `gemini = "/v1beta"`

如果某个网关需要自定义前缀，就显式写出来：

```json
{
  "providers": {
    "shared": {
      "auth": {
        "protocol_paths": {
          "openai": "/api/provider/openai/v1",
          "anthropic": "/api/provider/anthropic/v1",
          "gemini": "/api/provider/google/v1beta"
        }
      }
    }
  }
}
```

### `credential`

用于登录态 / OAuth / refresh token：

```json
{
  "providers": {
    "github-copilot": {
      "auth": {
        "mode": "credential",
        "issuer": "github_copilot",
        "credential": {
          "type": "oauth",
          "issuer": "github_copilot",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000
        }
      }
    }
  }
}
```

字段：

- `issuer`
- `credential`

注意：credential 模式下不接受 `base_url`、`protocol_paths`、`api_key`、`api_key_env`。

`credential` 必须带 issuer 信息，这样运行时才能知道这份 credential 是谁的，例如：

- `openai_chatgpt`
- `github_copilot`
- `gitlab`
- `atomgit`

### `bedrock_sigv4`

用于 AWS 原生签名：

```json
{
  "providers": {
    "bedrock": {
      "auth": {
        "mode": "bedrock_sigv4",
        "base_url": "https://bedrock-runtime.us-east-1.amazonaws.com",
        "region": "us-east-1",
        "profile": "prod"
      }
    }
  }
}
```

### `google_adc`

用于 Vertex / Google ADC。和 `api` 一样，它也需要一个共享入口的 `base_url`；区别只是凭证来源来自 Google ADC，而不是 API key。

```json
{
  "providers": {
    "vertex": {
      "auth": {
        "mode": "google_adc",
        "base_url": "https://us-central1-aiplatform.googleapis.com",
        "protocol_paths": {
          "openai": "/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
        }
      }
    }
  }
}
```

### `sap_ai_core`

用于 SAP AI Core。

## adapter 与 auth 的关系

auth 决定身份来源；adapter 决定协议。

同一个 auth 可以服务多个 adapter，只要运行时组合合法。

例如：

- `github_copilot` credential 可以配 `openai` adapter
- `github_copilot` credential 也可以配 `anthropic` adapter
- `atomgit` credential 可以配 `openai` adapter，运行时使用 AtomGit 的 OpenAI-compatible gateway
- `openai_chatgpt` credential 只适合 `openai` adapter 且 `backend = "chatgpt_codex"`
- `bedrock_sigv4` 只适合 `amazon_bedrock`
- `sap_ai_core` 只适合 `openai`

如果配置了错误组合，运行时报配置错误即可，不再为旧结构做兼容转换。

## 常见示例

### OpenAI API

```json
{
  "execution": {
    "provider": "openai",
    "adapter": "openai",
    "model": "gpt-5",
    "agent": "build"
  },
  "providers": {
    "openai": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-5"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://api.openai.com",
        "api_key_env": "OPENAI_API_KEY"
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-5": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

### ChatGPT Codex OAuth

```json
{
  "providers": {
    "chatgpt": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-5.3-codex"
      },
      "auth": {
        "mode": "credential",
        "issuer": "openai_chatgpt",
        "credential": {
          "type": "oauth",
          "issuer": "openai_chatgpt",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000,
          "account_id": "acct-123"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "backend": "chatgpt_codex",
          "models": {
            "gpt-5.3-codex": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

### GitHub Copilot OpenAI

```json
{
  "providers": {
    "github-copilot": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-4o-mini"
      },
      "auth": {
        "mode": "credential",
        "issuer": "github_copilot",
        "credential": {
          "type": "oauth",
          "issuer": "github_copilot",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-4o-mini": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

### AtomGit OAuth

```json
{
  "providers": {
    "atomgit": {
      "defaults": {
        "adapter": "openai",
        "model": "Kimi-K2-Instruct"
      },
      "auth": {
        "mode": "credential",
        "issuer": "atomgit",
        "credential": {
          "type": "oauth",
          "issuer": "atomgit",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000,
          "account_id": "atomgit-user"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "Kimi-K2-Instruct": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

AtomGit 的默认模型列表流程会对齐 AtomCode：先按 `Max -> Pro -> Lite`
调用 CodingPlan `claim-v2`，再用命中的 tier 请求 `models-v2`。如果需要覆盖
请求身份，可以在 HTTP adapter 上设置 `user_agent`；否则 AtomGit credential
会默认使用 AtomCode 风格的 User-Agent。内置默认 User-Agent 使用固定的官方
产品版本字符串，不使用 agena 名称或版本；其他 header 继续放在
`extra_headers`。

### GitHub Copilot Anthropic

```json
{
  "providers": {
    "github-copilot-claude": {
      "defaults": {
        "adapter": "anthropic",
        "model": "claude-sonnet-4"
      },
      "auth": {
        "mode": "credential",
        "issuer": "github_copilot",
        "credential": {
          "type": "oauth",
          "issuer": "github_copilot",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000
        }
      },
      "adapters": {
        "anthropic": {
          "enabled": true,
          "auth_header": "authorization",
          "auth_scheme": "Bearer",
          "extra_beta_header": "interleaved-thinking-2025-05-14",
          "models": {
            "claude-sonnet-4": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

### Shared Multi-Adapter Provider

```json
{
  "providers": {
    "shared": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-4.1-mini"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://gateway.example.com",
        "api_key_env": "SHARED_GATEWAY_API_KEY",
        "protocol_paths": {
          "openai": "/v1",
          "anthropic": "/v1",
          "gemini": "/v1beta"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-4.1-mini": {
              "enabled": true,
              "thinking_modes": {
                "deep": {
                  "thinking": {
                    "type": "effort",
                    "effort": "high"
                  }
                }
              }
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "models": {
            "claude-sonnet-4": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

这里：

- `openai` 会走 `https://gateway.example.com/v1`
- `anthropic` 会走 `https://gateway.example.com/v1`
- `gemini` 如果启用，会走 `https://gateway.example.com/v1beta`

### Provider-Routed Shared Gateway

```json
{
  "providers": {
    "provider_gateway": {
      "defaults": {
        "adapter": "openai",
        "model": "gpt-4.1-mini"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://api.cxits.cn",
        "api_key_env": "CX_API_KEY",
        "protocol_paths": {
          "openai": "/api/provider/openai/v1",
          "anthropic": "/api/provider/anthropic/v1",
          "gemini": "/api/provider/google/v1beta"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "gpt-4.1-mini": {
              "enabled": true
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "models": {
            "claude-sonnet-4": {
              "enabled": true
            }
          }
        },
        "gemini": {
          "enabled": true,
          "models": {
            "gemini-2.5-pro": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

这里不再需要回退和猜测。共享根路径就是 `https://api.cxits.cn`，其余协议前缀由 `protocol_paths` 显式给出：

- `openai` -> `/api/provider/openai/v1`
- `anthropic` -> `/api/provider/anthropic/v1`
- `gemini` -> `/api/provider/google/v1beta`

### Amazon Bedrock SigV4

```json
{
  "providers": {
    "bedrock": {
      "defaults": {
        "adapter": "amazon_bedrock",
        "model": "anthropic.claude-3-7-sonnet-20250219-v1:0"
      },
      "auth": {
        "mode": "bedrock_sigv4",
        "base_url": "https://bedrock-runtime.us-east-1.amazonaws.com",
        "region": "us-east-1",
        "profile": "prod"
      },
      "adapters": {
        "amazon_bedrock": {
          "enabled": true,
          "models": {
            "anthropic.claude-3-7-sonnet-20250219-v1:0": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

### Cline API Subscription

```json
{
  "providers": {
    "cline_api": {
      "defaults": {
        "adapter": "openai",
        "model": "cline-pass/qwen3.7-max"
      },
      "auth": {
        "mode": "api",
        "base_url": "https://api.cline.bot",
        "api_key_env": "CLINE_API_KEY",
        "protocol_paths": {
          "openai": "/api/v1"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "api_mode": "chat",
          "models_url": "https://api.cline.bot/api/v1/ai/cline/recommended-models",
          "models": {
            "cline-pass/qwen3.7-max": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

这里有两个和普通 OpenAI-compatible gateway 不一样的点：

- 聊天请求走 `https://api.cline.bot/api/v1/chat/completions`，所以 `base_url` 是根 `https://api.cline.bot`，`protocol_paths.openai` 显式写 `/api/v1`
- 订阅模型列表不依赖标准 `/models`，而是把 `models_url` 指到 `https://api.cline.bot/api/v1/ai/cline/recommended-models`

Web 设置页里可以直接在 `Provider Auth` 区输入 API key；它会自动创建 `cline_api` provider preset，不需要手填 `base_url`。

## 迁移后的结论

现在 provider 相关配置应理解为：

- provider 是逻辑入口
- auth 只管身份与认证
- adapter 是协议实现
- model key 是真实上游模型名
- provider 默认模型由 `defaults.adapter` 和 `defaults.model` 分别指定
- 外部运行请求也应分别传 `provider_id`、`adapter_id`、`model_id`
