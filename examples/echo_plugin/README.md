# `echo_plus` Sample Plugin

这是一个最小但完整的 Agena dynamic plugin 示例，覆盖了四类能力：

- 自定义 tool：`echo_plus`
- `before_tool` hook：重写输入并覆盖 pending title
- `after_tool` hook：修改 title / output / payload metadata
- `shell_env` hook：为 `bash` 注入环境变量

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

可以把 `target/release/` 目录直接配置到 Agena：

```toml
[plugins]
paths = ["examples/echo_plugin/target/release"]
```

也可以显式指向某个动态库文件：

```toml
[plugins]
paths = ["examples/echo_plugin/target/release/libagena_echo_plugin.so"]
```

`paths` 是相对配置文件目录解析的。

## 示例行为

`echo_plus` 的输入 schema：

```json
{
  "message": "hello",
  "uppercase": false,
  "tags": ["demo", "sample"]
}
```

执行顺序：

1. `before_tool` 会把 `message` 改成 `"[prepared] hello"`，并把 pending title 改成 `Echo Plus (prepared)`。
2. `invoke_tool` 会生成文本输出和结构化 payload。
3. `after_tool` 会继续修改 title / output_text / payload，并写入元数据。
4. 任意 `bash` 调用会收到：
   - `AGENA_SAMPLE_PLUGIN=echo_plus`
   - `AGENA_SAMPLE_PLUGIN_CWD=<当前 cwd>`
