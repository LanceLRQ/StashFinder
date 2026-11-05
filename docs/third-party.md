# 第三方来源

简体中文 | [English](third-party.en.md)

M0/M1 代码独立实现，未复制 FindX 或 goz 源码。两者是设计调研参考，不作为运行依赖。后续若引入源码，需逐文件保留原版权与许可声明。

直接依赖版本由 manifests 和锁文件确定。下表记录来源和许可，不替代发布包中完整的传递依赖许可清单。

| 依赖 | 来源 | 许可 |
|---|---|---|
| Tauri / tauri-build / API / CLI | [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 |
| windows / windows-core | [microsoft/windows-rs](https://github.com/microsoft/windows-rs) | MIT OR Apache-2.0 |
| windows-service | [mullvad/windows-service-rs](https://github.com/mullvad/windows-service-rs) | MIT OR Apache-2.0 |
| Tokio | [tokio-rs/tokio](https://github.com/tokio-rs/tokio) | MIT |
| rusqlite | [rusqlite/rusqlite](https://github.com/rusqlite/rusqlite) | MIT |
| SQLite（bundled） | [sqlite.org](https://sqlite.org/copyright.html) | Public domain |
| serde / serde_json | [serde-rs](https://github.com/serde-rs) | MIT OR Apache-2.0 |
| anyhow | [dtolnay/anyhow](https://github.com/dtolnay/anyhow) | MIT OR Apache-2.0 |
| tempfile | [Stebalien/tempfile](https://github.com/Stebalien/tempfile) | MIT OR Apache-2.0 |
| pcre2 / pcre2-sys | [BurntSushi/rust-pcre2](https://github.com/BurntSushi/rust-pcre2) | Unlicense OR MIT |
| PCRE2 | [PCRE2Project/pcre2](https://github.com/PCRE2Project/pcre2) | BSD-3-Clause |
| chrono | [chronotope/chrono](https://github.com/chronotope/chrono) | MIT OR Apache-2.0 |
| uuid | [uuid-rs/uuid](https://github.com/uuid-rs/uuid) | MIT OR Apache-2.0 |
| React / React DOM | [facebook/react](https://github.com/facebook/react) | MIT |
| TypeScript | [microsoft/TypeScript](https://github.com/microsoft/TypeScript) | Apache-2.0 |
| Vite / React plugin | [vitejs](https://github.com/vitejs) | MIT |
| Vitest | [vitest-dev/vitest](https://github.com/vitest-dev/vitest) | MIT |
| Testing Library React | [testing-library/react-testing-library](https://github.com/testing-library/react-testing-library) | MIT |
| jsdom | [jsdom/jsdom](https://github.com/jsdom/jsdom) | MIT |
| React 类型定义 | [DefinitelyTyped](https://github.com/DefinitelyTyped/DefinitelyTyped) | MIT |
| Prettier | [prettier/prettier](https://github.com/prettier/prettier) | MIT |

实现依据包括 Microsoft 的 [Shell 数据传输](https://learn.microsoft.com/en-us/windows/win32/shell/dragdrop)与[命名管道访问控制](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)、[Tauri 配置](https://v2.tauri.app/reference/config/)和 [SQLite 发布记录](https://sqlite.org/changes.html)。依赖锁定后仍需按发布节奏检查修复与许可变化。
