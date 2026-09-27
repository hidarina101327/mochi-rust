# DeskBox 功能整合与来源边界

本文以 DeskBox 公共仓库提交 `fdb5d45a5f83005c5d7a765afb2d83648fdd97ad`（2026-09-26 扫描基线）和当前 Mochi 源码为参照。实现采用 Mochi 自己的数据模型、Rust/Win32 边界和界面资源；没有复制 DeskBox 源码、图标、图片或品牌资产。

## 上游许可证与历史边界

该基线根目录 [LICENSE](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/LICENSE) 声明 **GPL-3.0-only**。仓库在提交 `60116f304b0054b2b48f49639e735d6e799a7b63`（2026-07-01）调整许可证，并加入 [LICENSE_CHANGE.md](https://github.com/Tianyu199509/DeskBox/blob/60116f304b0054b2b48f49639e735d6e799a7b63/LICENSE_CHANGE.md)。该说明把此前 MIT 发布的版本与变更后的 GPL 版本区分开。

- 最后扫描到的 MIT 正式版本：v1.1.10，提交 `581ebb3c35fea08f4dedbc75b85074a1a95f9305`；该提交的 [LICENSE](https://github.com/Tianyu199509/DeskBox/blob/581ebb3c35fea08f4dedbc75b85074a1a95f9305/LICENSE) 为 MIT。
- GPL 边界提交：`60116f304b0054b2b48f49639e735d6e799a7b63`；首个包含该变更的正式版本 v1.2.0，提交 `d1ab43294dc43f443da42b2ef877c4bbd7ac5c03`，其 [LICENSE](https://github.com/Tianyu199509/DeskBox/blob/d1ab43294dc43f443da42b2ef877c4bbd7ac5c03/LICENSE) 为 GPLv3。

以上是对仓库公开文件和历史的记录，不是法律意见或对每个历史文件来源的独立鉴定。当前主线代码不应被复制或改写后并入 Mochi；若要复用历史 MIT 文件，仍须逐文件核实来源和依赖许可证。

## 上游行为与源码入口

DeskBox 的文件格子可指向托管收纳目录或既有目录；它支持目录导航、多种排序、显示叠放、文件拖放和 Shell 文件操作。叠放是展示状态，不等同于磁盘子目录。桌面整理先预览移动计划；自动整理则按规则处理符合条件的新文件。相关架构和行为入口：

| 关注点 | DeskBox 扫描入口 |
|---|---|
| Widget 宿主和内容边界 | [current_architecture.md](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/docs/architecture/current_architecture.md) |
| Widget 配置和目录归属 | [WidgetConfig.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Models/WidgetConfig.cs)、[WidgetManager.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/WidgetManager.cs) |
| 文件格子、映射文件夹和用户操作 | [FileSurfaceContent.xaml](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Controls/WidgetContents/FileSurfaceContent.xaml)、[FileSurfaceContent.xaml.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Controls/WidgetContents/FileSurfaceContent.xaml.cs)、[FileService.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/FileService.cs) |
| 排序、监听与拖放/叠放契约 | [WidgetViewModel.SortingAndWatchers.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/ViewModels/WidgetViewModel.SortingAndWatchers.cs)、[file_drag_stack_contract.md](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/docs/architecture/%5B重要勿删%5Dfile_drag_stack_contract.md) |
| 搜索、天气、音乐 | [SearchEngineService.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/SearchEngineService.cs)、[EverythingSearchService.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/EverythingSearchService.cs)、[WeatherService.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/WeatherService.cs)、[MusicSessionService.cs](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/MusicSessionService.cs) |

## Mochi 当前实现

工作区的虚拟目录配置仍由 [mapped_folders.rs](rust/crates/mochi-core/src/mapped_folders.rs) 管理，桌面卡片 Folder 则是独立的本地文件入口；二者用途和配置生命周期不同。

| Mochi 能力 | 当前源码入口与范围 |
|---|---|
| Folder 数据协议、导航、排序和限额 | [desktop_cards.rs](rust/crates/mochi-core/src/desktop_cards.rs)、[folder.rs](rust/crates/mochi-core/src/desktop_cards/folder.rs)、[providers.rs](rust/crates/mochi-core/src/desktop_cards/providers.rs)：可选本地根目录、直接子目录导航、父级/根级返回、过滤、隐藏项、网格/列表、名称/修改时间/类型/手动排序、按类型分组及用户叠放；目录视图状态有界保存。每次扫描最多 4096 项、显示最多 500 行，应用快照另有共享读取预算。 |
| 文件操作与整理 | [folder_operations.rs](rust/crates/mochi-core/src/desktop_cards/folder_operations.rs)、[folder_auto.rs](rust/crates/mochi-core/src/desktop_cards/folder_auto.rs)、[folder_actions.rs](rust/crates/mochi-app/src/app/desktop_cards/folder_actions.rs)、[desktop_files.rs](rust/crates/mochi-app/src/platform/desktop_files.rs)：新建目录、改名、导入/粘贴复制或移动、复制/剪切、回收站、外部拖入/拖出、整理计划预览后确认执行、可选的新文件自动整理。移动和复制经过限额、计划及暂存校验；卡片内叠放和排序不创建磁盘目录。QuickLook 仅在用户已运行该程序时可调用。 |
| Folder 界面和窗口交互 | [desktop_window/folder.rs](rust/crates/mochi-app/src/desktop_window/folder.rs)、[desktop_window/painting.rs](rust/crates/mochi-app/src/desktop_window/painting.rs)、[desktop_window.rs](rust/crates/mochi-app/src/desktop_window.rs)：键盘与多选、上下文操作菜单、分组折叠、合并/拆分页、单卡片胶囊状态；卡片宿主及布局在 [desktop_window.rs](rust/crates/mochi-app/src/desktop_window.rs)。 |
| Weather | [weather.rs](rust/crates/mochi-core/src/desktop_cards/weather.rs)：用户设置城市；通过 Open-Meteo 地理编码和预报 API 获取当前、最多 12 小时和 7 天数据，带 15 分钟缓存、8 秒单次网络超时、60 秒失败退避及陈旧缓存错误状态。 |
| Music | [desktop_media.rs](rust/crates/mochi-app/src/platform/desktop_media.rs)、[utilities.rs](rust/crates/mochi-app/src/app/desktop_cards/utilities.rs)：Windows GSMTC 当前会话/来源，支持播放暂停、上一首、下一首和前后跳转 10 秒。 |
| Search | [desktop_search.rs](rust/crates/mochi-app/src/platform/desktop_search.rs)、[utilities.rs](rust/crates/mochi-app/src/app/desktop_cards/utilities.rs)：搜索卡片合并 Mochi 工作区全文搜索与 Everything IPC 文件结果，可打开结果；Everything 不可用时显示来源错误。 |

卡片级 To-do、QuickNote/捕获、Clock、快捷项等 Mochi 模块仍是 Mochi 自身功能，可与上述页面并列使用；它们不代表已完整实现 DeskBox 的聚合搜索或 Glance 皮肤。

## 当前边界与剩余差距

Mochi 已在卡片内形成文件格子的浏览、选择、基础管理、拖放和整理流程。仍未达到 DeskBox 的若干产品边界：

- 卡片是 Mochi 的原生浮动窗口，不是 Explorer 桌面图标表面或 Explorer 内嵌窗口；没有完整 Widget Group、多胶囊栏和其跨屏编排模型。单卡片胶囊/边缘抽屉只是 Mochi 窗口行为。
- 文件条目菜单是 Mochi 自己的操作菜单，不是完整调用 Windows Shell 扩展的系统“更多上下文菜单”。Windows 文件拖放、回收站及文件传输走 Mochi 的 Shell 适配；外部 Shell 菜单和属性页尚无同等入口。
- Search 当前聚合工作区匹配与 Everything 文件路径，不含 DeskBox 的应用、系统设置、Todo、Quick Capture 等统一类别、查询语法、历史/收藏和批量结果动作。Everything 需用户自行安装并运行。
- Weather 采用 Open-Meteo 单一来源，没有 DeskBox 的 MSN fallback/皮肤组合；Music 当前支持控制常用传输动作和选源，不包括等同的音量、重复/随机模式和完整封面布局。
- QuickLook 预览依赖用户已运行 QuickLook；Mochi 不安装该依赖。天气需联网，媒体控制需 Windows 可用会话。

对于“在 Mochi 里映射并整理一个目录”的路径，主要操作已落地；对于“在 Windows 桌面/Explorer 中直接管理图标并获得 DeskBox 的完整窗口组与 Shell 体验”，仍需另做平台集成，不能以现有 Folder 卡片宣称完全等同。

## 独立实现与复用边界

可复用 Mochi 已有的桌面卡片宿主、行快照/动作模型、工作区全文搜索、Win32 Shell 文件操作、窗口拖放及 WinRT 媒体适配。新增行为应继续留在 Mochi 自有实现中：Folder 配置与目录状态放 core，真实文件变更经限额/重验/用户确认后交给 app Shell 边界；Weather/Search 数据源以适配器暴露错误；Music 仅在用户动作时控制系统会话。不得复制 DeskBox 现行 GPL 文件或其 UI/资源；该段只描述可观察行为和本地能力，不表示协议或法律兼容性结论。

## 验证状态

2026-09-26 在 Windows 本机验证：

- `cargo build --offline --locked -p mochi-app` 成功。开发预览程序已复制到 `rust/target/deskbox-preview/mochi-app.exe`，并用该程序实际启动离屏桌面管理器截图验证入口；构建仍有 4 条既存的未使用导入警告。
- `cargo test --offline --locked -p mochi-core -p mochi-app desktop --no-fail-fast`：177 项通过（app 102、core 75），默认跳过 8 项需单独执行的测试。
- 单独执行 Shell smoke：临时目录中的真实 `IFileOperation` 复制、内容校验和移动后来源进入回收站通过，覆盖含空格和表情字符的文件名。
- 单独执行天气 smoke：真实 Open-Meteo Shenzhen 地理编码、当前天气、12 小时和 7 日预报，以及重复读取缓存通过。中文“深圳”的地理编码也已核对。
- 单独执行 GSMTC 只读 smoke：成功读取本机媒体会话；没有通过测试切换用户正在播放的媒体。播放、跳曲和跳转命令已编译并接入，未对当前播放器逐项执行。
- Everything Query2 IPC 编码与边界解析测试通过；本机未运行 Everything，未验证真实 Everything 索引结果。
- 原生离屏渲染测试通过并检查 14 张图片，覆盖列表/网格、窄窗、明暗主题、多选叠放、胶囊及天气/音乐/搜索设置。图片位于 `rust/target/desktop-verification/deskbox/`；内容卡片图片使用固定展示夹具，不能作为网络或媒体实际数据的证明。
- Rust 格式检查与原生模块大小检查通过（293 个模块，检查器另有 4 项测试通过）。

这些检查覆盖此次集成的主要行为；不等于完成上文所列 DeskBox 差异功能的验收。
