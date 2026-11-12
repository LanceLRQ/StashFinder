# 开发与验证

简体中文 | [English](development.en.md)

本工程提供正式桌面外壳、分区登记、NTFS 自动索引与增量更新、任务管理和离线文件搜索。默认自动更新已登记分区，未登记分区不扫描；没有 USN 日志时等待用户选择扫描方式或单独授权创建。“索引…”仍可手动校验。M0 文件桥接验证入口仅在前端开发构建中显示，与真实搜索结果分开。

NTFS 卷身份查询需要只读访问句柄，不能使用仅用于设备属性的零访问句柄。VHD 实机验收包含这一查询；自动检查不能替代实机流程。

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

在仓库根目录以普通权限启动开发界面及后台：

```powershell
npm --prefix ui run dev:desktop
```

启动器先等待前台后台握手，再运行 Tauri。开发目录为 `.local/dev-assets`。若默认验证管道上已有兼容后台，则复用连接。退出时只停止本次启动的后台。需要独立调试时，可以分别运行 `cargo run -p stashfinder-service -- --foreground --data-dir "$PWD\.local\dev-assets"` 和 `npm --prefix ui run tauri -- dev`。

启动进入固定在最左侧的概览标签。双击分区卡片打开目录浏览。顶部菜单打开设置标签。多个搜索标签在同一桌面窗口内独立运行。概览右栏显示待处理问题、更新阶段和暂停状态，可直接处理或展开最近操作。选中分区后，右栏只显示该分区的详情和操作。取消按任务 ID 定位，取消后分区暂停，需显式继续更新。普通用户前台模式可能无权读取物理磁盘布局，界面应明确显示不可访问；完整设备验收需使用下述 LocalSystem 服务。“设置 → 关于与诊断”可检查实际应用版本、后台模式和连接状态。命名管道限定配置账户、SYSTEM 和管理员，拒绝远程连接；IPC 不接受数据库路径或任意执行命令。

## 正式 Windows 安装包

从仓库根目录运行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-installer.ps1`。生成的 NSIS 安装包位于 `target/release/bundle/nsis/`。

安装时保留默认的 `Program Files\StashFinder` 位置。首次安装默认由桌面管理后台。打开应用时可选择“启动索引后台”或“稍后启动”。启动时按 Windows 提示确认 UAC，退出应用后后台随之停止。勾选安装 Windows 服务后，安装器会注册并启动自动运行的 `StashFinder` 服务，关闭界面后服务继续运行。服务停止时，打开界面或在“关于与诊断”重新连接会尝试启动。两种模式下界面都以普通权限运行，共用受保护的 `%ProgramData%\StashFinder` 和原安装账户。

退出桌面后重新运行安装包可切换模式。安装器记住上次选择。静默安装用 `/S /SERVICE` 或 `/S /NOSERVICE` 显式指定，单独使用 `/S` 沿用已保存的选择。卸载移除服务注册并保留索引目录。实机验收应分别核对桌面退出后会话后台停止、服务模式下关闭桌面后服务继续运行，以及切换后索引仍可读。

## 桌面界面与设置验收

- 从概览输入关键词并按 Enter 新建搜索标签。打开两个搜索标签，分别修改筛选和排序、滚动到不同位置，来回切换确认各页状态保持。完整结果按视口分页加载，零结果提示位于结果区中央。
- “登记分区”打开独立弹窗，关闭不改变概览的连接筛选。服务断开时保留已显示资产并报错，不把它们全部改成离线。
- 设置支持默认路径匹配。路径匹配偏好用于下次打开的搜索页，不覆盖当前搜索选择；修改后重启验证恢复。
- 偏好保存在 Tauri `app_config_dir` 的 `preferences.json`（Windows 通常为 `%APPDATA%\io.github.lancelrq.stashfinder\preferences.json`），与服务索引库分开。写入采用同目录临时文件替换，保存失败保留原配置并显示错误；不会更改系统启动项。
- 配置格式损坏或版本不匹配时明确报错。备份后移走该配置并重启可恢复默认设置；不要修改索引库来重置界面。
- 服务未启动时仍可进入设置并修改界面偏好；在“关于与诊断”重新连接，不把读取失败显示成已连接。
- 验证初始 1280×800、最小 1000×650 窗口及 Windows 100%／125%／150%／200% 缩放。自动组件测试不能替代原生窗口和真实后台验证。

## 目录浏览与标签页验收

- 从概览点击分区名称，或在文件页选择已登记分区。目录树按需读取本地索引，离线时同样可展开；双击文件夹或按 Enter 进入，面包屑和 Alt+↑ 返回上级。
- 用“+”或 Ctrl+T 新建浏览标签页，Ctrl+Tab 切换，Ctrl+W 关闭。每页保留位置、滚动和多选；Ctrl/Shift 点击与方向键选择，Ctrl+A 选择整个目录的直接子项。改分区名或盘符不改变目录定位。
- 在浏览页输入查询或从概览搜索，每次打开新搜索标签。也可使用“文件 → 新建搜索标签页”或 Ctrl+Shift+F。Ctrl+F 聚焦当前搜索框。关闭搜索标签后，若来源标签仍在，则返回来源标签。搜索结果右键“在新标签页浏览所在目录”通过文件系统与目录 ID 定位，不依赖源盘在线。
- 顶部“查看”菜单和文件页“视图”控制目录树、属性预览和面包屑。目录与预览窗格可从底部角落拖动调宽。预览只展示索引元数据，未知值明确显示未知，不读取文件内容。标签页和窗格状态保留在当前窗口会话。
- 目录每页最多 100 项，并受 16 KiB 帧预算限制；“加载更多”读取后续页面；搜索定位目标若在后续页，需继续加载查看。F5 刷新当前目录，索引版本变化后拒绝追加旧页并提示刷新；读取失败保留已显示结果，不能把错误显示成空目录。目录被确认删除时回退到最近可用祖先并清空选择。
- 用同一构建的普通权限 GUI 与后台验证在线和离线分区：展开多级目录、两个浏览页交替选中和滚动、进入搜索再返回、从结果定位父目录、折叠窗格。自动测试与受控浏览器数据不能替代此项真实 IPC 验收。

## 正式文件操作验收

快捷键、可用状态与限制见 [文件操作](file-operations.md)。桌面以普通权限执行，服务的选择接口只读；协议 7 增加分批选择，分区库版本 4 独立保存扫描得到的原生身份。旧扫描库更新身份前仍可搜索，但不能据旧路径直接操作文件。

构建后先获取普通 GUI 用户的 SID，再在管理员 PowerShell 中执行以下命令，目录必须全新，且严格限定为可丢弃测试盘：

```powershell
target/debug/stashfinder-service.exe --sid
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-REPLACE-WITH-USER-SID' -FileOperationsVhdDirectory 'D:\tmp\StashFinder-M6-new-directory' -HoldSeconds 1800
```

该流程复用临时 LocalSystem 服务，在新建的 1 GiB VHD 中建立 NTFS/exFAT/FAT32 三个分区和独立样本，不创建 USN 日志。只登记测试分区，生成普通文件、中文文件、目录、空目录、长路径及冲突样本。`verification.json` 的 `ready` 为 true 后，普通权限 GUI 才开始交互检查。等待期间可创建测试目录下的 `stop-gui` 标记提前结束；脚本卸载自己的 VHD，外层停止移除临时服务，保留镜像、独立数据目录和证据。失败时先检查报告，不复用资料盘。`-FileOperationsPhase Resume` 仅用于已生成且卷标仍匹配的该组测试分区，需新的空测试目录库。

逐项核对：完整多选及同目录原生菜单；复制／拖出到 Explorer 与逐文件内容、源保留；应用和 Explorer 间双向剪切粘贴、跨卷和外部实体来源；冲突、取消和部分结果；F2 与仅大小写改名；Delete 回收后仅还原测试项；Shift+Delete 默认取消和明确确认；离线与身份变更拒绝、盘符变化与后台重启；输入框／IME 与关闭时的执行提示。不要清空整个回收站，不用“目标接受”代替目标文件检查。长路径要分别验证“长源文件复制到短目标”和“深层目录保留层级复制”，后者可能触发 Windows 限制。

另有默认忽略的真实 IPC／原生接口检查。仅在已授权并就绪的上述专用 VHD 上运行；它会生成自己的样本，验证三种文件系统的身份、批量跨卷复制、移动、大小写改名反馈及永久删除，不代替 GUI 验收：

```powershell
$env:SF_M6_MANIFEST = 'D:\tmp\StashFinder-M6-new-directory\verification.json'
cargo test -p stashfinder-service --test file_operations_real registered_three --locked -- --ignored --nocapture
Remove-Item Env:SF_M6_MANIFEST
```

三分区测试后台和 VHD 就绪后，可在管理员 PowerShell 中复验快速重挂的监控恢复。下面的命令仅复用该清单对应的已授权测试盘，不安装或重启服务，不创建日志；会短暂以只读方式重挂，再恢复读写，并仅对新生成的样本执行新建、改名和删除。每一步必须在 45 秒内自动反映到索引，且服务 PID 和登记身份保持不变：

```powershell
.\scripts\verify-scan-remount.ps1 -Manifest 'D:\tmp\StashFinder-M6-new-directory\verification.json'
```

结果保存为测试目录下的 `remount-verification.json`；清理仍由原验证后台包装器负责。该验证不替代真实 USB 硬件异常拔出验收。

`file_operations_real` 中的 `registered_failures` 用例默认忽略，只能在同一已授权测试环境中显式运行。它检查取消、改名冲突、文件占用、路径替换和实际部分复制；遇“文件正在使用”对话框时需要选择取消，因此不能无人值守运行全部 ignored 用例。

## 分区登记验收

- 在概览点击“登记分区”，弹窗首次打开不勾选任何分区。选择需要管理的分区并命名；没有选择的分区不登记，也不扫描文件。
- 已登记分区按物理盘分组，显示当前挂载点、文件系统、容量及采集时间。改名只改变软件内名称，不修改卷标。
- 后台约每 5 秒检查设备，在线 NTFS 约每 2 秒检查变化；设备响应与正在执行的任务会影响延迟。离线容量是历史快照；未获取到的值显示未知。
- 身份冲突、证据不足和库异常会显示原因，不自动关联不确定设备。格式化后旧记录保留为历史，新文件系统需再次登记；手动移除历史入口尚未提供。
- 退出、重开界面和服务，登记 ID、名称及历史容量应保留。ready 子库丢失/损坏不会被空库替换，其他分区仍可用。

开发模式必须显式指定绝对数据目录。SCM 模式默认 `%ProgramData%\StashFinder`，需预先创建；目录必须直接位于 ProgramData 下、禁用根目录 ACL 继承，仅允许 SYSTEM 和 Administrators 完全控制并向子项继承，不能包含重解析点。GUI 通过服务读取数据，不直接打开数据库。下面的验证脚本会创建独立的受保护目录。正式安装与迁移流程尚未提供。

`catalog.sqlite` 保存登记，`volumes/<filesystem-id>.sqlite` 保存独立文件系统数据，`tasks.sqlite` 保存任务、自动更新策略和暂停状态。SQLite 使用 WAL/FULL；不能只复制活动的主数据库文件作为备份。恢复接口仅生成单独的候选资产库，尚无完整备份恢复工具。协议版本为 8，单帧上限为 16 KiB，资产、任务、目录条目和搜索结果分页读取；界面和服务需使用同一次构建。旧分区库经归属核验后事务迁移；归属不符的库不会被迁移。

## 自动更新验收

- “设置 → 索引与同步”默认开启自动更新。关闭须确认，已在执行的任务继续；手动索引仍可使用。设置保存失败保留原策略。
- 重连验证身份与日志范围，正常时增量回放。增量重试从已保存游标继续，游标失效时等待用户确认重建，旧活动代次继续提供搜索。一次变更页读取失败不推进该页游标，不用读取失败推断删除。
- 单工作线程串行处理索引，空日志检查不创建任务或重载缓存；失败退避并最多自动尝试 3 次，重连或手动重试可恢复。例行自动增量更新和扫描成功后清除执行记录。其他已结束任务合计保留最近 30 条，每个分区另保留最新未解决问题。搜索和目录按索引版本刷新。
- 暂停会停止当前任务后续批次，保留已提交数据；增量恢复从游标继续，全量构建恢复时重新开始。服务停止或设备拔出可能先等当前系统调用返回。
- 在管理员终端执行以下命令前，明确授权脚本创建、重建日志、格式化和修改这块新建测试 VHD；脚本拒绝已有目录，结束后卸载测试盘并移除临时服务，保留报告和测试数据。

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-your-normal-user-SID' -SyncVhdDirectory 'D:\tmp\StashFinder-M3-new-directory' -HoldSeconds 600
```

脚本覆盖自动建库、目录改名与硬链接、增量删除、策略与暂停、SCM 重启、重挂换盘符、日志重建及格式化历史。报告为 `.local/sync-verification.json`。VHD 不替代 USB 硬件异常拔出和系统休眠测试。

无开发服务器的构建与启动：

```powershell
npm --prefix ui run tauri -- build --debug --no-bundle
& .\target\debug\stashfinder-desktop.exe
```

## NTFS 索引与离线搜索

1. 登记一个可访问的 NTFS 分区。有日志时，“索引…”默认使用已有 USN 日志。检测到没有日志后，可在概览右栏点击“选择索引方式”，默认选中“基础扫描”。
2. “基础扫描”不创建日志，开始扫描前建立监控；扫描期间变化会触发重新校验，显示“扫描快照”。“创建 USN 日志并索引”需要单独勾选授权；新日志最大 32 MiB、分配增量 8 MiB，已有日志保持原配置。
3. 建库依次显示 NTFS 记录读取、元数据读取及扫描期变更补齐。可取消。失败或取消保留旧活动索引；首次失败时，能读取的部分数据标为“未完成记录”。读取错误不会被视为文件删除。
4. 进入“文件搜索”。默认不区分大小写并包含路径，`aaa` 可命中 `aaa\bbb.exe`，`aaa bbb` 要求两词都出现且不限制词序，`"aaa bbb"` 匹配连续文本。可切换仅文件名和分区范围。盘符条件仅匹配当前在线挂载。
5. 卸载测试盘并刷新设备，再搜索或重启服务。结果应保留分区名、卷内路径、索引时间与离线状态。在线也只表示分区当前状态，不保证快照中的文件尚未变化。

搜索返回完整计数、全局排序与有界批量页，支持类型、正则和元数据查重，语法见搜索帮助；真实结果的操作见文件操作帮助。重解析点只记录节点，不遍历目标；硬链接保存多个路径。文件内容不读取。大规模搜索缓存与无命中查询仍需后续性能优化。

构建代次目前保留在分区库内，反复重建会增加数据库体积；自动回收旧代次尚未提供。设备无法访问或身份待确认时，健康的本地快照仍可查询，连接异常不会被显示为在线。

需完整实机验证时，在管理员 PowerShell 中运行以下命令。该命令明确授权在**新建的可丢弃测试 VHD** 内创建日志、生成、改名和删除样本：

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-填入普通账户SID' -IndexVhdDirectory 'D:\tmp\StashFinder-M2-全新目录' -HoldSeconds 600
```

脚本只接受不存在的目录，创建 512 MiB VHDX 和 NTFS 测试分区，用独立目录枚举对照中文、零字节、硬链接、长路径、重解析点及元数据，再验证扫描期改名/写入/删除、取消、卸载和 SCM 重启搜索。报告为 `.local/index-verification.json`，包含本次样本规模、建库耗时、无命中查询耗时与服务工作集；这些小规模调试构建测量不能代表百万/千万文件性能。成功后的等待期用于普通权限 GUI 验收，测试盘此时已离线。结束或失败均卸载并清理临时服务，镜像和受保护数据保留。

## 扫描模式与容量验收

exFAT/FAT32 登记后自动扫描。无日志 NTFS 在“索引…”选择基础扫描；选择持久化，服务重启后仍不创建日志。NTFS 的扫描保留文件引用与硬链接关系；exFAT/FAT32 使用当前文件系统实例内的相对路径标识，不把改名前后路径当成稳定文件 ID。

在线监控使用 [FindFirstChangeNotificationW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstchangenotificationw) 合并变化后重新遍历元数据；它不提供持久增量日志。空闲时不重复建库，每 15 分钟进行兜底校验；重连、恢复自动更新及服务重启重新校验。读取失败或监控失效不发布新代次、不确认删除，旧索引继续可查；首建失败的已保存记录明确为部分结果。持续变化会有限重试，失败原因在概览右栏展示。扫描模式会反复读取整个已登记分区，大盘耗时及旧代次空间占用仍需后续优化。

分区显示总量、已用、剩余和剩余比例；容量采集失败仍保留原快照和时间，并单独标记失败，不把在线分区误报为离线，也不阻止健康索引更新。物理容量仍单独显示，未分配空间不算分区剩余。

以下管理员验收仅操作指定新建 VHD：创建 1 GiB 镜像和三个 256 MiB NTFS/exFAT/FAT32 分区，测试中文、零字节、长路径、在线变更、SCM 重启、卸载重挂与离线容量；不创建 USN 日志。

```powershell
.\scripts\verify-service.ps1 -OwnerSid 'S-1-5-21-your-normal-user-SID' -ScanVhdDirectory 'D:\tmp\StashFinder-M4-new-directory' -HoldSeconds 600
```

报告为 `.local/scans-verification.json`。等待期可用普通权限窗口验证三种分区；在测试目录创建 `show-offline` 文件可切换为离线界面验收，创建 `stop-gui` 文件可提前结束等待。脚本结束后卸载镜像并移除临时服务，测试文件与目录库保留；这不替代真实 USB 异常断开、休眠和大规模性能验收。

## 文件桥接验收

先进入“开发验证”。以下操作只用于可丢弃样本。

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

脚本将已构建的服务程序复制到受保护的 `%ProgramFiles%/StashFinder-M0-Verification/`，新建 `%ProgramData%/StashFinder-Verification-<随机ID>/`，以 LocalSystem 手动启动临时服务 `StashFinderM0`，检查状态，等待 120 秒供普通权限 GUI 连接，然后停止、移除服务及本次生成的程序。既有同名服务或程序目录会导致拒绝执行。数据目录保留供核对，不复用开发数据库。此脚本不是正式安装程序，只发现设备基础信息，不扫描文件。

在等待期间，普通权限窗口“检查连接”应显示“Windows 后台服务”。关闭界面后用普通账户运行 `target/debug/stashfinder-service.exe --probe`，服务应仍可响应。结果写入忽略的 `.local/service-verification.json`。真实其他账户的拒绝访问和 Windows 10 验证需有相应测试环境。

需要验证换盘符和卸载重挂时，可显式添加 `-VhdDirectory 'D:\tmp\StashFinder-M1-全新目录'`。脚本只接受尚不存在的 D:\tmp 子目录，创建一块 512 MiB 动态 VHDX，并在新磁盘中格式化 NTFS/exFAT/FAT32 测试分区；验证选择子集、命名、盘符变化、离线容量、重挂及 SCM 重启。等待期供普通权限 GUI 验收，结束后卸载 VHD；测试镜像和受保护数据目录保留，不自动删除。报告在 `.local/assets-verification.json`。不要直接对资料盘执行这些挂载和格式化测试。

自动验证成功后，脚本另建一个 64 MiB NTFS 分区供 GUI 登记和改名，盘符见报告的 `guiMount`。只操作该测试分区；关闭重开界面后核对名称和登记数量。失败时同样保留指定等待期供诊断，随后执行卸载和服务清理；不要直接结束管理员脚本进程。

## M0 简化合成性能基线

```powershell
New-Item -ItemType Directory -Force .local | Out-Null
cargo build -p stashfinder-store --bin baseline --release --locked
.\target\release\baseline.exe .local\baseline-1m.sqlite 1000000 > .local\baseline-1m.json
.\target\release\baseline.exe .local\baseline-10m.sqlite 10000000 > .local\baseline-10m.json
```

目标数据库必须不存在；程序拒绝覆盖。每组包含 12 个合成分区、1200 个目录、中文/英文名称、重复名称与大小、空值元数据。SQLite 使用 WAL、FULL 同步、分批事务，结束后执行完整性检查；关系缓存从重开的只读数据库加载。

输出包括导入、重开、缓存载入、磁盘字节数、私有提交/工作集/峰值工作集；5 类查询各重复 20 次，报告前 200 条的 P95，并对照 SQL 与缓存结果。查询把空格分隔词作为字面 AND 子串；它不是完整搜索语法实现。跨路径边界的查询使用完整路径保守匹配。

导入后系统文件缓存仍热，不能称为冷启动；测量不覆盖真实硬盘索引、完整生产字段、全局排序、查重、正则、取消、GUI 或服务空闲预算。运行时避免并发构建和其他重任务。`.local/` 是验证产物，不入库。

## 真实 NTFS 首次索引基准

`cargo build --release -p stashfinder-service --example index-benchmark --locked` 构建测量入口。在管理员终端运行 `target\release\examples\index-benchmark.exe <卷GUID路径> <新的数据库绝对路径> 600 [auto|legacy|sequential]`。目标卷必须已有 USN 日志。工具以只读方式访问显式指定的卷，并创建独立数据库。

`legacy` 使用目录扫描。`sequential` 强制使用顺序 MFT 读取，分别记录读取、解析、对象写入、路径导入、回放和搜索加载耗时。`auto` 允许在确认读取能力不足时改用旧路径，日志记录实际策略与原因。超时采用协作取消。

生产后台首次建库或重建有有效 USN 日志的 NTFS 分区时使用 `auto`。确认读取能力不足或初始父目录关系不完整时，后台清理候选代次，再完整执行一次旧扫描。基础扫描继续使用目录读取和变化监控。

在管理员终端使用 `scripts/benchmark-ntfs-index.ps1 -Binary <基准程序绝对路径> -Volume <卷GUID路径> -Output <另一卷上的新目录>` 运行交替 A/B。使用 Node.js 24 或更新版本运行 `node scripts/compare-ntfs-index.mjs <旧路径库> <顺序路径库> <新建差异JSONL>`，核对文件身份、完整路径和元数据。运行期间避免编译及其他重任务。实际写盘的隔离 VHD 验收入口为 `scripts/verify-ntfs-sequential.ps1`，执行前按目标授权。

对比其他工具时，应统一分区范围和元数据选项，使用空库，核对完整路径及元数据，并单独记录可搜索时间与持久化时间。未清除系统文件缓存时不得称为冷盘基准。NTFS 目录元数据可能滞后于文件句柄查询，特别是硬链接与仍在写入的文件；数量相同不代表元数据完全一致。

分区库版本 5 增加对象到路径的索引，保留既有记录和游标。后台与桌面端应一起升级；旧程序不支持打开升级后的库。SQLite 继续使用 WAL 和 FULL 同步，页缓存按需增长、每连接目标上限为 64 MiB；这不是整个后台的内存上限。

## 布局

`crates/core` 定义身份、查询模型与协议；`crates/windows` 提供设备与 NTFS 适配；`crates/service` 协调登记、索引和查询任务；`crates/store` 提供资产目录、独立分区库、搜索缓存及合成基线；`ui` 提供资产管理、只读搜索与开发验证入口。

## 完整搜索验收

搜索语法见 [搜索帮助](search.md)。验证类型编辑重启恢复、跨分区组合查重、零与未知大小、正则错误、全局列排序、远距离滚动与跨页选择、取消旧查询。协议版本为 8，批量页仍遵守 16 KiB 上限。百万／千万生产查询缓存的复现步骤与限制见 [查询基线](query-performance.md)。

## 图标、缩略图和缓存

见 [系统图标与缩略图](system-visuals.md)。普通权限桌面端获取系统图像，后台只读解引用条目；协议 8 增加图片记录接口和搜索结果修订号。图片经过桌面桥接返回，不经过后台的 16 KiB 帧，也不存入分区数据库。专项测试和在线／离线 GUI 验证步骤见该文档。

## 便携构建

运行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-portable.ps1` 生成 release GUI、后台、白名单 ZIP 和 SHA256 清单。默认输出到忽略的 `.local/packages/`；已有输出目录会被拒绝。包验证入口为 `scripts/verify-portable-package.ps1 -Archive <zip路径>`，不执行包内程序。

`portable.json` 标记启用程序目录旁的 `data`，覆盖索引、界面设置、图片缓存及 WebView2 用户数据路径；GUI 仍以普通权限运行，后台每次单独提升，不注册服务。托盘和设置中的退出会撤销会话并等待后台停止，异常 GUI 退出另由父进程检测处理。没有标记则沿用现有模式。使用和依赖见[便携版说明](portable.md)。

专项回归：`cargo test -p stashfinder-windows --test portable --locked`、`cargo test -p stashfinder-service --test portable --locked`、`cargo test -p stashfinder-desktop runtime --locked`。服务集成测试仅在临时目录建立空登记，不注册或扫描资料盘；真实 UAC、托盘和目录搬迁另行实机验收。
