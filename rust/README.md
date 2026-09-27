# Mochi 

Windows 桌面应用，使用 Win32 消息循环、Direct2D 绘制和 DirectWrite 文本排版。业务数据保存在本地工作区；磁盘协议与 Electron 实现保持兼容。

## 工作区

| Crate | 职责 |
| --- | --- |
| `mochi-blocks` | 文档块、稳定引用、修改审批；可选 Git 同步和检索 |
| `mochi-core` | 工作区、文件、索引、AI、日程、工作流及命令行宿主 |
| `mochi-app` | Windows 窗口、输入、布局、绘制及后台任务协调 |
| `mochi-installer` | IExpress 安装启动器 |

依赖方向为 `mochi-app → mochi-core → mochi-blocks`；应用也直接使用块模型。安装启动器独立于业务层。

## 构建与运行

完整工作区需要 Windows、Rustup、Visual Studio Build Tools 的 C++ 桌面工具和 Windows SDK。`rust-toolchain.toml` 固定 Rust 1.88.0，并安装 rustfmt 和 Clippy。从仓库根目录执行：

```powershell
cd rust
cargo build --locked -p mochi-app
cargo run --locked -p mochi-app
```

必须保留上级目录的 `shared/` 和 `tests/fixtures/`，其内容被源码和测试通过 `include_str!` 引用。仅复制本目录无法完成构建与测试。

开发构建缺少 `build/icon.ico` 时会发出警告。制作发行包还需要 Node.js、根目录 JavaScript 依赖、Playwright Chromium 和 Inno Setup 6 或更新版本：

```powershell
# 在仓库根目录安装图标生成工具及浏览器。
npm ci
npx playwright install chromium
# 在 rust/ 中调用打包入口。
cd rust
./tools/package-native.ps1
```

打包脚本生成图标，构建应用、工作流进程、浏览器剪藏宿主和社区命令行工具，并准备 Python 运行时、许可证及安装资源。参数见脚本的 `param` 声明；输出默认写入根目录 `release/`。发行包构建不等于 crates.io 发布。

## 检查

在 `rust/` 中执行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo test --workspace --doc --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
node tools/check-module-size.cjs
node --test tools/check-module-size.test.cjs
```

`--all-targets` 不包含 rustdoc 示例测试，因此单独执行 `--doc`。Clippy 存量告警尚未全部清理，不把运行成功表述为零告警。Windows 绘制、剪贴板和窗口交互测试可能需要交互桌面；失败时保留具体用例和环境信息。

最近一次完整检查（2026-09-22，Rust 1.88、Windows MSVC）为 2,705 通过、21 失败、13 忽略；应用测试中的文件树、审批与 Wiki 建议用例尚有失败。格式、Clippy、API 文档和 2 项文档示例测试通过。

项目使用 MIT 许可证，正文见 [LICENSE](../LICENSE)。第三方组件的独立声明位于 `crates/mochi-app/assets/licenses/`，不被项目许可证替代。
