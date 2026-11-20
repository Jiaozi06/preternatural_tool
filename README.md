# 绿玩变异工具箱

Windows 11 桌面工具，提供拾取、战斗、飞行、悬浮窗和在线更新设置。

## 构建

安装 Rust stable MSVC 工具链后，在 Windows 上运行：

```powershell
cargo build --release
```

可执行文件位于 `target/release/game-macro.exe`。发布版本使用文件名“绿玩变异工具箱 v1.2.exe”。

## 更新

设置页可选择“官方源”或“镜像源”，更新日志从仓库根目录的 `update.json` 获取。镜像源通过 gh-proxy.com 访问 GitHub Raw。

## 许可

本项目使用 MIT License，详见 [LICENSE](LICENSE)。
