本目录保存 2026-10-05 网页提取调研的独立 Rust 对照程序。它不调用 Agena tools，不启动 Agena，也不属于主 workspace。结论见 [调研与实施报告](../rust-crawling-improvements.md)，元数据见 [来源快照](../rust-crawling-sources.json)，结果见 [比较记录](../rust-crawling-comparison.json)。

五条对照链路固定为 `crw-extract 0.24.1`（完整复现旧 Agena 提取顺序）、`dom_smoothie 0.18.2 + htmd 0.5.5`、`dom_smoothie 0.18.2 + html-to-markdown-rs 3.17.0`、`trafilatura 0.3.0` 和 `rs-trafilatura 0.2.2`。依赖由本目录的 Cargo.toml 和 Cargo.lock 固定。Agena 新实现由主 workspace 的 `extraction_probe` example 调用相同生产提取函数，避免在研究程序里复制一个不同实现。

从仓库根目录重放十个人工样本：

```sh
python3 docs/research/rust-crawl-probe/prepare.py --output /tmp/agena-content-fixtures.json
cargo run --locked --manifest-path docs/research/rust-crawl-probe/Cargo.toml -- /tmp/agena-content-fixtures.json > /tmp/agena-content-baselines.jsonl
cargo run --locked -p agena-web --example extraction_probe -- /tmp/agena-content-fixtures.json > /tmp/agena-content-production.jsonl
```

人工 HTML 位于 [web-content.json](../../../tools/fixtures/web-content.json)。其中前三个来自原有样本，另有多篇回答、技术结构、短页面、相对链接、嵌套表格、中文问答、数学与折叠内容。正文回归还单独测试多 article 无 main、嵌套 main、片段链接、MIME、JavaScript 壳页、验证码误判、Unicode 和复杂度上限。

另外四个公开页面为 Rust 1.85 发布文章、中文 Rust 书、Tokio spawn 和 MDN async function。比较复用了之前抓取的 HTML，并非每次运行都重新下载。[public-pages.json](public-pages.json) 记录原始 URL、抓取时间、SHA-256 和事实标记。公开页面 HTML 和完整提取正文没有放进仓库；有历史快照时可重放：

```sh
python3 docs/research/rust-crawl-probe/prepare.py --html-dir /tmp/agena-rust-web-survey --output /tmp/agena-content-fixtures.json
```

也可重新抓取，遇到变化默认拒绝把新快照当成原报告的输入；接受变化需要显式 `--allow-changed`，脚本会记录新的哈希：

```sh
python3 docs/research/rust-crawl-probe/prepare.py --live --allow-changed --output /tmp/agena-content-fixtures.json
```

下载限制为 HTTPS、五次跳转、每页 30 秒和 5 MiB；这只是研究脚本。生产路径另外使用宿主许可、DNS 检查、robots 和响应预算。

生成可提交的紧凑比较记录：

```sh
python3 docs/research/rust-crawl-probe/summarize.py /tmp/agena-content-fixtures.json /tmp/agena-content-baselines.jsonl /tmp/agena-content-production.jsonl /tmp/agena-content-comparison.json
```

计数包括标题、Markdown 正文及单独的评论字段；只将 Markdown 转义下划线归一化，另保留仅正文的标记计数。公开页面输出保存 SHA-256、长度、事实标记和策略，人工样本保存完整输出，便于检查结构。**标记保留数不等于语义准确率、正文召回率或性能基准**。这里没有任何生产网站吞吐、成功率或反爬能力排名。
