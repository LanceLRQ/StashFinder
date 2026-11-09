# StashFinder 便携版

[English](portable.en.md)

适用于 Windows 10/11 x64。这是开发阶段的便携构建，尚未完成全部干净系统发布验收。

1. 完整解压 ZIP 到可写目录，例如 `D:\Tools\StashFinder`，不要直接在压缩包内运行。
2. 普通权限双击 `StashFinder.exe`；仅在启动索引后台时确认 Windows UAC。请勿将整个界面设为“以管理员身份运行”。
3. 在概览主动登记需要管理的分区。未登记分区不会扫描，不会自动导入开发机或安装式版本的数据。
4. 关闭窗口会驻留托盘，后台继续更新。单击托盘图标或选择“显示 StashFinder”恢复界面。
5. 要停止软件或搬迁目录，右键托盘图标选择“退出”，或使用“设置 → 关于与诊断 → 退出便携版”，等待后台结束后再移动整个目录。文件操作尚在执行时会阻止退出。

## 数据位置

所有应用持久数据位于程序旁的 `data`：

| 子目录 | 用途 |
|---|---|
| `index` | 登记、任务和独立分区索引 |
| `config` | 界面偏好与图片缓存额度 |
| `local` | 图片缓存、应用本地数据及 `EBWebView` 用户数据 |
| `runtime` | 实例锁与运行会话 |

首次启动才创建数据目录；原始 ZIP 不包含索引、硬盘登记或缓存。同一目录同时只允许运行一个实例。复制和备份目录前必须退出；运行中的 SQLite 主文件不能单独当作备份。

搬到其他电脑后，离线记录仍可搜索；在线硬盘需要重新核对身份和更新索引。当前缓存默认 2 GB，可在设置中调整或清空；清空不会删除索引或源文件。不要只带走 EXE，必须保留后台、`portable.json` 和整个 `data`。

## 系统依赖与边界

界面需要系统已安装 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。此轻量包不携带固定运行时，不会自动联网安装。缺少运行时时请先安装，再启动软件。

后台每次运行单独提升权限，不安装 Windows 服务，不添加登录启动项。其管道与本目录绑定，与安装式版本隔离。后台在正常退出或检测到 GUI 进程退出后停止；拒绝 UAC、目录不可写或重复启动都会报错，不会回退到其他数据目录。程序本身不提供跨电脑的管理员权限。

提升后的后台启用账户已有的备份读取权限，以枚举受保护目录；不授予新的账户权限、不修改源文件 ACL，也不启用还原写入权限。若系统策略未授予该读取权限，后台会报错。

包内 `manifest.json` 记录基础源码提交、是否包含未提交改动、文件大小及 SHA256；`built_from_source=false` 表示使用预先构建的二进制打包。ZIP 旁的 `.sha256` 用于核对完整归档。此开发包未签名；第三方来源见包内说明，完整发布许可和系统矩阵继续在发布阶段收口。

从源码打包：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-portable.ps1`。默认输出到 `.local/packages/<时间>/`；重复构建使用新输出目录，不覆盖已有包。预构建打包可使用 `-SkipBuild -BinaryDirectory <目录>`，该目录需含 `stashfinder-desktop.exe` 和 `stashfinder-service.exe`；脚本不运行这些预构建文件。验证归档使用 `scripts/verify-portable-package.ps1 -Archive <zip路径>`。
