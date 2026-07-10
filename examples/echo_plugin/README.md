# `echo` Sample Plugin

这是一个最小但完整的 Agena dynamic plugin 示例，覆盖了四类能力：

- 自定义 entry：`echo`
- `tool.execute.before` hook：重写输入并覆盖 pending title
- `tool.execute.after` hook：修改 output / payload metadata
- `shell_env` hook：为 `bash` 注入环境变量
- `provider.list` hook：注入一个示例 provider descriptor

示例使用新的插件层聚合宏写法：

- `#[agena_plugin(...)] impl EchoPlugin { ... }` 汇集插件元数据、tool handler、hook handler 和导出方式。
- `#[tool(...)]` 直接描述工具名、help、约束、展示策略和 handler；宏会生成隐藏 schema 与 dispatch glue。
- `#[arg(...)]` 可以写在方法参数上，宏会生成隐藏 input struct，并把 JSON 输入解析成结构化参数后再调用方法。
- `stream = ...`、`path(...)` 和 `network(...)` 写在目标 `#[tool(...)]` 上，宏会自动生成 stream / permission 分发。
- `#[hook(tool.before)]`、`#[hook(shell.env)]` 等显式 dotted DSL 描述 hook；返回 patch、`Option<Patch>`、`()`, `Result<_>` 都会自动适配。
- `#[derive(PluginConfigStore)]` + 字段级 `#[config(default)]` 汇集配置字段；`#[agena_plugin(..., config)]` 自动生成配置 schema，并在 init 时解析到 `PluginConfig<EchoPluginConfig>`。
- `export = cdylib` 在 cdylib crate 中自动导出 host 加载所需的动态库入口。

## 构建

在当前目录执行：

```bash
cargo build --release
```

产物在：

- Linux: `target/release/libagena_echo_plugin.so`
- macOS: `target/release/libagena_echo_plugin.dylib`
- Windows: `target/release/agena_echo_plugin.dll`

## 加载

在 `config.json` 中显式声明 cdylib plugin：

```json
{
  "plugins": {
    "list": {
      "example.echo": {
        "package": {
          "kind": "cdylib",
          "path": "examples/echo_plugin/target/release/libagena_echo_plugin.so"
        },
        "config": {
          "uppercase": false
        }
      }
    }
  }
}
```

macOS 和 Windows 的动态库文件名分别是：

- macOS: `examples/echo_plugin/target/release/libagena_echo_plugin.dylib`
- Windows: `examples/echo_plugin/target/release/agena_echo_plugin.dll`

`path` 相对 config 文件所在目录解析。配置后可以检查加载状态：

```bash
agena config validate
agena plugin status
agena plugin inspect echo
```

如果要在 debug 构建下测试，把 `path` 指向 `target/debug` 下的动态库：

```json
{
  "plugins": {
    "list": {
      "example.echo": {
        "package": {
          "kind": "cdylib",
          "path": "examples/echo_plugin/target/debug/libagena_echo_plugin.so"
        },
        "config": {
          "uppercase": true
        }
      }
    }
  }
}
```

## 示例行为

`echo` 的输入 schema：

```json
{
  "text": "hello"
}
```

执行顺序：

1. `tool.execute.before` 会把 `text` 改成 `"[prepared] hello"`，并把 pending title 改成 `Echo (prepared)`。
2. `tool.invoke` 会生成文本输出和结构化 payload。
3. `tool.execute.after` 会继续修改 output_text，并写入元数据。
4. 任意 `bash` 调用会收到：
   - `AGENA_ECHO=1`
   - `AGENA_ECHO_CWD=<当前 cwd>`

如果 `config.uppercase = true`，`tool.invoke` 会把输出转成大写。
