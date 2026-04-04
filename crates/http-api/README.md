# agena-http-api

`agena-http-api` 是从 `agena` core crate 拆分出来的可选 HTTP 后端。

它直接依赖 `agena`，提供：

- `axum` 路由与 `ApiServer`
- 面向聊天与管理界面的 HTTP 资源接口
- SSE 事件流
- keyset pagination 与消息懒加载

启动：

```bash
cargo run -p agena-http-api -- serve
```

查看命令帮助：

```bash
cargo run -p agena-http-api -- --help
```
