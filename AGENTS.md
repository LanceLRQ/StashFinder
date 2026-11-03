# StashFinder

面向 Windows 10/11 的个人硬盘文件目录管理与搜索工具，支持搜索已登记分区的离线文件记录。

A personal disk catalog and file search tool for Windows 10/11, including offline records of registered partitions.

## 约定 / Conventions

- 已确认技术路线：Rust 后台服务、Tauri 2 + React + TypeScript 界面、SQLite 持久化。界面以普通用户权限运行，Windows 原生接口与业务逻辑分离。
  Agreed stack: a Rust service, a Tauri 2 + React + TypeScript UI, and SQLite storage. Run the UI without elevation and separate Windows adapters from domain logic.
- 物理硬盘用于分组，分区是登记和管理单位。只扫描用户登记的分区，稳定身份与盘符分离。
  Group by physical disk and manage individual partitions. Scan only registered partitions; keep identity independent of drive letters.
- 索引不修改用户普通文件；创建 USN 日志需单独授权，不自动扩容。桌面文件操作首版仅提供复制语义。
  Indexing must not modify ordinary user files. Creating a USN journal requires separate consent; never enlarge it automatically. Initial desktop file operations use copy semantics only.
- 每个分区文件系统实例独立存储。文件变更、持久补齐任务和游标共同提交；读取失败不能作为文件已删除的证据。
  Store each partition filesystem instance independently. Commit file changes, pending metadata tasks, and the cursor together. A read failure is not evidence of deletion.
- 优先保证准确性与可恢复性。离线、历史、身份不确定和元数据缺失必须明确表达，不能以零值或成功状态掩盖。
  Prioritize correctness and recovery. Represent offline, historical, uncertain-identity, and missing-metadata states explicitly instead of disguising them as zero values or success.
- 开发先按任务规模说明方案和验收标准；只改任务所需内容。默认由主进程开发、审查和修复，派工需用户决定。
  Explain the approach and acceptance criteria in proportion to the task. Limit changes to the requested scope. The primary agent develops, reviews, and fixes changes unless the user authorizes delegation.
- Git 提交、推送、合并等写操作需用户明确指示。暂存和提交分开执行。
  Git writes, including commits, pushes, and merges, require explicit user instruction. Run staging and committing separately.

## 完成标准 / Done

尚未定义检查命令。项目有了之后写在这里。

No check command is defined yet. Write it here once the project has one.

每次交付运行与改动相符的检查，报告实际命令、结果和未验证范围。有测试框架后，功能和修复先建立行为测试；没有框架时使用可重复的检查方法，不虚构测试通过。

Run checks appropriate to each change and report the actual commands, results, and unverified scope. Once a test framework exists, establish behavioral tests before implementing features or fixes. Until then, use repeatable checks and never claim tests passed without evidence.

## 文档 / Docs

- 公开文档放在 `docs/`。
- 过程文档（research、discussion、specs、plans、audit、reference、ui）放在 `docs/_internal/<category>/`。该目录已被 git 忽略：其中的文件和内容不进入提交、拉取请求和公开文件。
- 索引是 `docs/_internal/README.md`。创建或查找过程文档之前先读它，新文件在其中登记。
- 工作流类 skill 产出的规格和计划放到 `docs/_internal/specs/` 和 `docs/_internal/plans/`，不论该 skill 默认写到哪里。
- 本机备注（当前阶段、硬约束、本机路径）在 `docs/_internal/LOCAL.md`。该文件存在时，开工前先读它。

- Public docs live in `docs/`.
- Process docs (research, discussion, specs, plans, audit, reference, ui) live in `docs/_internal/<category>/`. That directory is git-ignored: its files and their content stay out of commits, pull requests and public files.
- The index is `docs/_internal/README.md`. Read it before creating or looking for a process doc, and register new files in it.
- Specs and plans produced by workflow skills go to `docs/_internal/specs/` and `docs/_internal/plans/`, whatever path the skill uses by default.
- Local notes (current stage, hard constraints, paths on this machine) are in `docs/_internal/LOCAL.md`. When that file exists, read it before starting work.

## 语言 / Language

始终使用简体中文与用户沟通。公开文档提供中英双语版本，内部过程文档使用中文，代码注释使用英文。根目录的中文 README 与英文 README 保持内容一致；代码标识符、命令和路径保持原样。

Always communicate with the user in Simplified Chinese. Provide public documentation in Chinese and English, write internal process docs in Chinese, and write code comments in English. Keep the root Chinese and English READMEs aligned; preserve code identifiers, commands, and paths as written.
