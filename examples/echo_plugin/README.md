# `echo` Sample Plugin

这是一个最小但完整的 Agena dynamic plugin 示例，覆盖了四类能力：

- 自定义 entry：`echo`
- `tool.execute.before` hook：重写输入并覆盖 pending title
- `tool.execute.after` hook：修改 output / payload metadata
- `shell_env` hook：为 `bash` 注入环境变量
- `provider.list` hook：注入一个示例 provider descriptor

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

在 `config.toml` 中显式声明 cdylib plugin：

```toml
[plugins.list.echo]
kind = "cdylib"
path = "examples/echo_plugin/target/release/libagena_echo_plugin.so"
options = { uppercase = false }
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

```toml
[plugins.list.echo]
kind = "cdylib"
path = "examples/echo_plugin/target/debug/libagena_echo_plugin.so"
options = { uppercase = true }
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

如果 `options.uppercase = true`，`tool.invoke` 会把输出转成大写。
