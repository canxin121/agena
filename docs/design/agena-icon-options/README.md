# Agena 图标候选归档

本目录保留 Agena 早期图标方向的静态设计稿。正式采用的是 **01：切口 A / Shift**；02–08 只作为设计参考，不会被应用、Web 构建或安装包加载。

![Agena 图标候选对比](overview.png)

| 编号 | 方向 | 视觉特点 | 文件 |
| --- | --- | --- | --- |
| 01 | 切口 A / Shift（正式） | 暖橙色圆角底，圆顶 A，斜向开口形成前进感 | [SVG](01-shift.svg) · [1024](01-shift-1024.png) · [32](01-shift-32.png) · [64](01-shift-64.png) |
| 02 | 小写 a / Soft A | 薄荷绿，圆润亲和 | [SVG](02-soft-a.svg) · [1024](02-soft-a-1024.png) · [32](02-soft-a-32.png) · [64](02-soft-a-64.png) |
| 03 | AG 组合 / AG | 深色底，橙色 A 与米白 G 交叠 | [SVG](03-ag-monogram.svg) · [1024](03-ag-monogram-1024.png) · [32](03-ag-monogram-32.png) · [64](03-ag-monogram-64.png) |
| 04 | 星核 / Nova | 紫色底，四角星，中心三角形负空间 | [SVG](04-nova.svg) · [1024](04-nova-1024.png) · [32](04-nova-32.png) · [64](04-nova-64.png) |
| 05 | 轨道 / Orbit | 深色底，橙色星球与倾斜米白轨道 | [SVG](05-orbit.svg) · [1024](05-orbit-1024.png) · [32](05-orbit-32.png) · [64](05-orbit-64.png) |
| 06 | 折纸 / Fold | 米白底，橘红折面组成向上的 A 形 | [SVG](06-fold.svg) · [1024](06-fold-1024.png) · [32](06-fold-32.png) · [64](06-fold-64.png) |
| 07 | 回旋 / Flow | 蓝色底，三段旋转弧线 | [SVG](07-flow.svg) · [1024](07-flow-1024.png) · [32](07-flow-32.png) · [64](07-flow-64.png) |
| 08 | 小伙伴 / Companion | 暖黄底，猫形角色 | [SVG](08-companion.svg) · [1024](08-companion-1024.png) · [32](08-companion-32.png) · [64](08-companion-64.png) |

每款包含：

- 可编辑的 SVG 源文件。
- 1024 × 1024 PNG。
- 32 × 32 和 64 × 64 PNG 预览。

图标由矢量路径绘制，不依赖字体或外部图片。本目录没有自动生成脚本；PNG 是静态导出文件。修改候选 SVG 后，需要手动重新导出对应尺寸。

## 与正式图标的关系

正式图标源文件是：

```text
packages/agena-web/src/assets/agena.svg
```

正式的平台资源由
`packages/agena-web/scripts/generate-app-icons.mjs` 生成到
`packages/agena-web/public/`。生成、检查和完整资源清单见
[Shift 图标说明](../agena-icon.md)。

本目录只用于回顾设计选择，不参与运行时资源、图标一致性检查或发布构建。
