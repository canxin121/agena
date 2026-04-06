# Agena TeX 文档

本目录放的是给 `Agena` 仓库准备的两份 LaTeX 中文文档：

- `agena-architecture.tex`
  - 面向小白的架构文档
- `agena-process.tex`
  - 面向小白的流程文档
- `shared/agena-preamble.tex`
  - 两份文档共享的页面样式、颜色、TikZ 节点样式和提示框样式

## VS Code

仓库根目录已经补好了 `.vscode/settings.json` 和 `.vscode/extensions.json`：

- 推荐扩展：`James-Yu.latex-workshop`
- 默认配方：`latexmk (xelatex)`
- 输出目录：当前 TeX 文件所在目录下的 `build/`
- 保存时自动编译

打开方式建议：

1. 用 VS Code 打开仓库根目录 `agena/`
2. 安装推荐扩展
3. 直接打开 `docs/tex/agena-architecture.tex` 或 `docs/tex/agena-process.tex`
4. 按 `Ctrl+Alt+B` 或保存文件触发编译

## 命令行编译

在仓库根目录执行：

```bash
latexmk -xelatex -outdir=docs/tex/build docs/tex/agena-architecture.tex
latexmk -xelatex -outdir=docs/tex/build docs/tex/agena-process.tex
```

清理输出：

```bash
latexmk -c -outdir=docs/tex/build docs/tex/agena-architecture.tex
latexmk -c -outdir=docs/tex/build docs/tex/agena-process.tex
```

## 为什么使用 XeLaTeX

- 文档是中文说明，`xelatex` 对中文更稳
- TikZ 图较多，`latexmk` 可以自动多轮编译
- VS Code 的 `latex-workshop` 对这套配置兼容性最好

## 输出位置

PDF 会写到：

- `docs/tex/build/agena-architecture.pdf`
- `docs/tex/build/agena-process.pdf`
