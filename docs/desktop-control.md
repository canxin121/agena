# 桌面应用控制

`agena.desktop` 通过 Cua Driver 查看和操作 Agena 服务进程所在机器的桌面。插件支持 Windows、Linux 和 macOS，使用同一套窗口、截图和输入工具。

## 本机驱动

安装与你的系统匹配的 Cua Driver。Agena 使用 Cua Driver 0.62.1 验证了集成接口。Agena 会从服务进程的 `PATH` 查找 `cua-driver`，也可以在启动服务前把 `AGENA_DESKTOP_DRIVER` 设为驱动可执行文件的绝对路径。驱动按照 Agena 的请求启动，并随工具调用结束关闭。 [Cua Driver 安装](https://cua.ai/docs/cua-driver/quickstart)

| 系统 | 桌面要求 |
| --- | --- |
| Windows | 在已登录的交互式用户会话中运行 Agena。目标程序需要处于驱动可访问的权限级别。 |
| Linux | 在桌面用户的图形会话中运行 Agena。X11 需要对应的 `DISPLAY` 和 X11 授权。Agena 同时启用 Cua Driver 的原生 Wayland 后端，窗口截图与输入取决于桌面合成器及其 Portal 权限。 |
| macOS | 在已登录的图形会话运行 Agena，并为 macOS 识别为负责进程的应用授予屏幕录制及辅助功能权限。 |

不同桌面环境的可用操作可查看 [Cua Driver 平台说明](https://cua.ai/docs/cua-driver/concepts/platform-support)。

## 会话权限

在插件工作台配置 `allowed_applications`，填写 Cua Driver `list_windows` 返回的 `app_name`，按名称精确匹配。只用查看和截图时保留 `allow_input: false`。需要点击、滚动和键盘输入时，设置 `allow_input: true`，再在当前会话的工具权限中配置 `agena.desktop.action`。桌面工具可在直接 Agena 会话中使用。

```json
{
  "agena.desktop": {
    "package": {"kind": "static"},
    "settings": {
      "allowed_applications": ["ExampleApp"],
      "allow_input": true
    }
  }
}
```

1. `desktop.windows` 列出已授权的可见窗口并返回当前会话可使用的 `window_id`。
2. `desktop.screenshot` 返回窗口图片及图片、窗口的尺寸。截图缩放后，按尺寸比例换算点击位置。
3. `desktop.action` 接受 `click`、`scroll`、`type` 和 `key`。坐标以目标窗口左上角为原点。驱动可能暂时将目标窗口带到前台。键盘修饰键 `meta` 对应 macOS 的 Command、Windows 的 Windows 键和 Linux 的 Super。
