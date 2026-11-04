# Third-party sources

[简体中文](third-party.md) | English

M0 is independently implemented. No FindX or goz source code has been copied. They are design research references, not runtime dependencies. Any future source reuse must retain original copyright and license notices per file.

Manifests and lockfiles determine exact direct dependency versions. This table records sources and licenses; it does not replace the full transitive license inventory required for release packages.

| Dependency | Source | License |
|---|---|---|
| Tauri / tauri-build / API / CLI | [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 |
| windows / windows-core | [microsoft/windows-rs](https://github.com/microsoft/windows-rs) | MIT OR Apache-2.0 |
| windows-service | [mullvad/windows-service-rs](https://github.com/mullvad/windows-service-rs) | MIT OR Apache-2.0 |
| Tokio | [tokio-rs/tokio](https://github.com/tokio-rs/tokio) | MIT |
| rusqlite | [rusqlite/rusqlite](https://github.com/rusqlite/rusqlite) | MIT |
| SQLite (bundled) | [sqlite.org](https://sqlite.org/copyright.html) | Public domain |
| serde / serde_json | [serde-rs](https://github.com/serde-rs) | MIT OR Apache-2.0 |
| anyhow | [dtolnay/anyhow](https://github.com/dtolnay/anyhow) | MIT OR Apache-2.0 |
| tempfile | [Stebalien/tempfile](https://github.com/Stebalien/tempfile) | MIT OR Apache-2.0 |
| React / React DOM | [facebook/react](https://github.com/facebook/react) | MIT |
| TypeScript | [microsoft/TypeScript](https://github.com/microsoft/TypeScript) | Apache-2.0 |
| Vite / React plugin | [vitejs](https://github.com/vitejs) | MIT |
| Vitest | [vitest-dev/vitest](https://github.com/vitest-dev/vitest) | MIT |
| Testing Library React | [testing-library/react-testing-library](https://github.com/testing-library/react-testing-library) | MIT |
| jsdom | [jsdom/jsdom](https://github.com/jsdom/jsdom) | MIT |
| React type definitions | [DefinitelyTyped](https://github.com/DefinitelyTyped/DefinitelyTyped) | MIT |
| Prettier | [prettier/prettier](https://github.com/prettier/prettier) | MIT |

Implementation references include Microsoft's [Shell data transfer](https://learn.microsoft.com/en-us/windows/win32/shell/dragdrop) and [named pipe access control](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights), [Tauri configuration](https://v2.tauri.app/reference/config/), and [SQLite release history](https://sqlite.org/changes.html). Locked dependencies still need periodic checks for fixes and license changes.
