# 安全策略

## 报告漏洞

请通过私密渠道报告，不要在公开 issue 里描述细节。在仓库公开维护者
联系方式之前，可发送至维护者私邮邮箱（见 `Cargo.toml` 的 `authors`
字段或 git log）。

收到报告后我们会：

1. 24 小时内确认收到
2. 评估影响面与利用难度
3. 协调披露时间窗（默认 90 天）
4. 修复后在 `CHANGELOG.md` 与 GitHub Security Advisory 公示

## 涉及凭据的最佳实践

- API key / token 不要直接写进 `config.toml`。优先使用：
  1. OS keyring（`store_backend = "keyring"`）
  2. 环境变量（`api_key_env = "ANTHROPIC_API_KEY"`）
- 配置中的 `api_key` 字段使用了 redacted `Debug`，但 `Serialize`
  仍保留原值——不要把 resolved config 直接 dump 到日志或公开渠道
- 插件签名验证：`agena-plugin-host` 的 `signing` feature 启用 ed25519
  校验；分发渠道应固定使用签名版本

## 依赖安全

- `cargo audit` 与 `cargo deny advisories` 在 CI 强制运行
- 任何 `RUSTSEC-*` 警告必须修复或在 `deny.toml` ignore 列表加注释说明

## 已知风险

- 工具执行（`session/tool/`）默认有 permission 门控；若运行 agena 时
  使用 `mode = "allow"`，等同于授予 LLM 完整 shell 访问
- `agena-api-server` 默认无认证；部署到非本地环境时必须前置反向代理
  + 鉴权层
