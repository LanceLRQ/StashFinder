# 开发与验证

简体中文 | [English](development.en.md)

本工程目前只提供 M0 验证窗口和合成数据测量，不会登记、枚举或索引现有硬盘。后台状态中的登记数固定为 0。不要将此版本作为个人文件目录库使用。

## 环境与构建

- Windows x64；目标 Windows 10/11。开发机成功不等于两个系统均已验收。
- Rust 1.94.0（`rust-toolchain.toml` 固定，包含 rustfmt/Clippy）。
- Node.js 和 npm；本轮开发环境为 Node 26.13.0、npm 11.20.0。
- MSVC C++ 构建工具、Windows SDK 和 WebView2，见 [Tauri 官方前置条件](https://v2.tauri.app/start/prerequisites/)。
- `Cargo.lock` 和 `ui/package-lock.json` 必须保留，首次依赖下载需要网络。

在仓库根目录执行：

```powershell
npm ci --prefix ui
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

统一检查包含 Rust 格式、Clippy（warning 视为失败）、测试与构建，以及前端 Prettier、TypeScript、组件测试和 Vite 构建。运行前关闭验证窗口和前台后台进程，避免 Windows 锁定待构建程序。生成物放在 `target/`、`ui/dist/`，不入库。

可用内存较少时，先在当前终端执行 `$env:CARGO_BUILD_JOBS = '1'`，再运行检查。首次全量编译 Windows/Tauri 依赖的内存需求较高，单任务编译可降低并行内存占用。

## 运行验证窗口

终端一，普通用户运行后台验证进程：

```powershell
cargo run -p stashfinder-service -- --foreground
```

终端二：

```powershell
npm --prefix ui run tauri -- dev
```

点击“检查连接”应显示“前台验证进程”。这条路径复用服务通信循环，但不等同于已经安装 Windows 服务。关闭界面不停止该后台进程；在终端一用 Ctrl+C 停止。命名管道限定配置账户、SYSTEM 和管理员，拒绝远程连接；没有磁盘扫描、任意文件操作或执行命令的 IPC。

无开发服务器的构建与启动：

```powershell
npm --prefix ui run tauri -- build --debug --no-bundle
& .\target\debug\stashfinder-desktop.exe
```

## 文件桥接验收

1. 点击“准备验证样本”，默认生成两个文件、一个含文件的目录及一个超过 260 字符路径的文件。它们位于本机应用数据目录 `io.github.lancelrq.stashfinder/m0-samples/`，路径在窗口中显示，退出后保留。
2. 点击第一行，按住 Shift 点击第三行（Ctrl 可逐项选择，Ctrl+A 全选）。按 Ctrl+C 或右键“复制”，到一个空的测试目录粘贴。核对文件内容和目录内文件，并确认源仍存在。方向键可选择条目，Esc 取消选择。
3. 在另一个空目录，按住已选中的文件行，直接拖到资源管理器后松开。应保留整组选中项且只提供复制效果；拖动期间按 Esc 应取消。先测试单个 `second.txt`，再测试多文件和目录。
4. 单独验证长路径样本，记录源路径长度、系统版本、目标是否接受及提示。Shell 版本或目标应用的限制必须报告，不能把它当成成功或静默忽略。
5. 删除一个生成的样本，再尝试操作，应显示不可用错误。这里只能删除本次生成样本，不能用真实用户资料测试。

文件桥接使用 Unicode `CF_HDROP` 绝对路径列表及 `Preferred DropEffect=COPY`。自动测试核对路径、用 Windows `IFileOperation` 将四类样本实际复制到临时目录，并检查目标内容和源文件。这不能替代资源管理器端到端验收。现阶段 GUI 只接受后台生成的样本 ID，不接收任意文件路径。生产版的分区身份重核验留待实际登记模型实现。

接口依据：[Shell 文件传输格式](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard)、[IFileOperation::CopyItems](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-copyitems)。全局结果可跨目录，不能直接套用有扁平条目限制的 [BHID_DataObject](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemarray-bindtohandler)。

从打包应用启动开发程序时，AppData 可能被重定向到启动器的 `Packages/.../LocalCache`，导致外部资源管理器无法访问程序看到的逻辑路径；这是已实测的验证环境限制。参见 [Windows 打包应用的文件系统行为](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)。应从普通终端启动验收，或设置 `STASHFINDER_M0_SAMPLE_ROOT` 为已经准备好的外部 M0 样本目录（如 `D:\tmp` 下的独立目录）。窗口只读取这组固定样本，不覆盖内容；缺少文件或类型不符会报错。这不是任意目录登记或扫描功能。

```powershell
$env:STASHFINDER_M0_SAMPLE_ROOT = 'D:\tmp\已准备的M0样本目录'
& .\target\debug\stashfinder-desktop.exe
```

剪贴板跨进程测试需要显式执行 `cargo test -p stashfinder-windows --test clipboard -- --ignored`，会替换系统剪贴板，不能与手工验收并行。同一执行环境中的测试不能替代外部资源管理器验收。

### 扩展操作验证（仅限可丢弃样本）

右键菜单另提供剪切、重命名、移入回收站、永久删除和刷新。此处验证 Windows 桥接能力，不代表正式版文件操作范围已经扩大。

- `Ctrl+X`：发布 `Preferred DropEffect=MOVE`，此时源文件仍在。到资源管理器的空目录粘贴，核对内容及源路径消失，再回窗口按 `F5`；不能仅凭“已剪切”消息判定移动成功。
- `F2`：单项重命名，输入完整名称（含扩展名）；拒绝路径、非法名称和已有目标。核对旧路径消失、新路径内容不变及列表更新。
- `Delete`：确认后请求移入回收站，核对回收站条目并还原，内容应不变。如 Windows 提示无法回收而要永久删除，请取消。
- `Shift+Delete`：独立确认后永久删除，不经过回收站。先取消一次核对源仍在，再对指定测试文件确认，核对源及回收站；不可对个人资料测试。
- `F5`：重新检查固定样本的可用性。移动或删除后保留“不可用”条目供核对，不代表正式索引同步；还原后再刷新可恢复可用状态。重启窗口仍要求初始固定样本完整，请另建一组样本开展下一轮测试。

重命名与删除使用 [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags)。取消或部分失败会报错并刷新，不能当成整组成功。拖出仍只允许复制。

## Windows 服务验证

先停止前台验证进程。在普通账户终端执行 `target/debug/stashfinder-service.exe --sid`，记下该 GUI 使用账户的 SID。在管理员 PowerShell 中执行：

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-填入普通账户SID' -HoldSeconds 120
```

脚本将已构建的服务程序复制到受保护的 `%ProgramFiles%/StashFinder-M0-Verification/`，以 LocalSystem 手动启动临时服务 `StashFinderM0`，检查状态，等待 120 秒供普通权限 GUI 连接，然后停止、移除服务及本次生成的受保护程序。既有同名服务或目录会导致拒绝执行，不能覆盖。此脚本不是正式安装程序，不扫描磁盘。

在等待期间，普通权限窗口“检查连接”应显示“Windows 后台服务”。关闭界面后用普通账户运行 `target/debug/stashfinder-service.exe --probe`，服务应仍可响应。结果写入忽略的 `.local/service-verification.json`。真实其他账户的拒绝访问和 Windows 10 验证需有相应测试环境。

## 合成性能基线

```powershell
New-Item -ItemType Directory -Force .local | Out-Null
cargo build -p stashfinder-store --bin baseline --release --locked
.\target\release\baseline.exe .local\baseline-1m.sqlite 1000000 > .local\baseline-1m.json
.\target\release\baseline.exe .local\baseline-10m.sqlite 10000000 > .local\baseline-10m.json
```

目标数据库必须不存在；程序拒绝覆盖。每组包含 12 个合成分区、1200 个目录、中文/英文名称、重复名称与大小、空值元数据。SQLite 使用 WAL、FULL 同步、分批事务，结束后执行完整性检查；关系缓存从重开的只读数据库加载。

输出包括导入、重开、缓存载入、磁盘字节数、私有提交/工作集/峰值工作集；5 类查询各重复 20 次，报告前 200 条的 P95，并对照 SQL 与缓存结果。查询把空格分隔词作为字面 AND 子串；它不是完整搜索语法实现。跨路径边界的查询使用完整路径保守匹配。

导入后系统文件缓存仍热，不能称为冷启动；测量不覆盖真实硬盘索引、完整生产字段、全局排序、查重、正则、取消、GUI 或服务空闲预算。运行时避免并发构建和其他重任务。`.local/` 是验证产物，不入库。

## 布局

`crates/core` 是通信协议；`crates/windows` 是 Windows 适配；`crates/service` 是后台入口；`crates/store` 是合成存储基线；`ui` 是验证窗口。M1 才引入正式的资产登记和独立分区数据库模式。
