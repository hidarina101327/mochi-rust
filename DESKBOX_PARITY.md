# DeskBox 功能对照

对照基线：DeskBox `fdb5d45a5f83005c5d7a765afb2d83648fdd97ad`；实现现况按本地 Mochi 源码核对。许可证、来源和本机测试记录见 [DESKBOX_INTEGRATION.md](DESKBOX_INTEGRATION.md)。

## 已有能力与可复用部分

| DeskBox 行为 | Mochi 当前对应能力 | 现状说明 |
|---|---|---|
| 托管/映射文件格子、目录浏览 | Folder 配置与快照：[folder.rs](rust/crates/mochi-core/src/desktop_cards/folder.rs)、[providers.rs](rust/crates/mochi-core/src/desktop_cards/providers.rs)；映射工作区虚拟目录：[mapped_folders.rs](rust/crates/mochi-core/src/mapped_folders.rs) | 桌面 Folder 可选本地路径、进入直接子目录、回上级/根目录；只枚举当前层。工作区目录映射是另一能力。 |
| 文件交互和整理 | [folder_operations.rs](rust/crates/mochi-core/src/desktop_cards/folder_operations.rs)、[folder_actions.rs](rust/crates/mochi-app/src/app/desktop_cards/folder_actions.rs)、[desktop_files.rs](rust/crates/mochi-app/src/platform/desktop_files.rs) | 多选、系统打开/资源管理器定位、复制剪切粘贴、外部导入、拖出、改名、新建子目录、回收站；整理先预览后确认；自动整理针对开启后新出现且稳定的文件。 |
| 分组、叠放和卡片组织 | [folder.rs](rust/crates/mochi-core/src/desktop_cards/folder.rs)、[desktop_window/folder.rs](rust/crates/mochi-app/src/desktop_window/folder.rs)、[folder_actions.rs](rust/crates/mochi-app/src/app/desktop_cards/folder_actions.rs) | 类型分组、手动顺序、显示叠放、合并/拆分纯 Folder 格子及分组折叠已接入。排序/叠放是展示配置，不移动磁盘文件。 |
| 卡片视图与胶囊 | [desktop_cards.rs](rust/crates/mochi-core/src/desktop_cards.rs)、[desktop_window/painting.rs](rust/crates/mochi-app/src/desktop_window/painting.rs)、[desktop_window.rs](rust/crates/mochi-app/src/desktop_window.rs)、[dock.rs](rust/crates/mochi-app/src/desktop_window/dock.rs) | 网格/列表、卡片分页、可收起单卡片胶囊、边缘抽屉。胶囊与边缘抽屉为两种状态。 |
| 天气 | [weather.rs](rust/crates/mochi-core/src/desktop_cards/weather.rs)、[utilities.rs](rust/crates/mochi-app/src/app/desktop_cards/utilities.rs) | 手动设城市；Open-Meteo 当前天气、逐小时、七日预报与本地缓存。 |
| 音乐 | [desktop_media.rs](rust/crates/mochi-app/src/platform/desktop_media.rs)、[utilities.rs](rust/crates/mochi-app/src/app/desktop_cards/utilities.rs) | Windows GSMTC 选播放来源、播放暂停、上一首/下一首、±10 秒。 |
| 搜索 | [desktop_search.rs](rust/crates/mochi-app/src/platform/desktop_search.rs)、[utilities.rs](rust/crates/mochi-app/src/app/desktop_cards/utilities.rs)、[workspace_search.rs](rust/crates/mochi-app/src/app/workspace_search.rs) | Search 卡片显示 Mochi 工作区全文匹配和 Everything 文件/目录结果；可打开结果。Everything 不可用时显示错误。 |
| To-do、捕获和时间 | Mochi 既有桌面卡片模块及捕获窗口 | 可继续直接复用 Mochi 自有 To-do、QuickNote/捕获、Clock 等能力；当前 Search 卡片没有把它们合并成 DeskBox 式统一搜索。 |

Folder 扫描每次最多检查 4096 个目录项、显示最多 500 行，另受卡片快照共享预算约束；隐藏敏感目录与 reparse/symlink 项不会因为显示隐藏项而绕过。文件正文不由 Folder provider 读取。传输配置另有条目、深度和字节上限。

## 仍未同等实现的 DeskBox 能力

| 缺口 | 对用户路径的影响 | 建议的独立实现方向 |
|---|---|---|
| Windows Desktop/Explorer 图标层、Explorer 嵌入与原生桌面组织 | Mochi 现在是在 Mochi 桌面卡片窗口中完成文件格子流程；若用户期望直接把 Explorer 桌面图标变成格子，核心入口仍不同。 | 将 Shell/Explorer 集成作为独立平台层评估，先证明窗口生命周期、多屏恢复、拖放/OLE 和 Explorer 重启行为，再决定是否实现。不要把它塞进 Folder provider。 |
| 多胶囊栏、Widget Group 及跨屏组合 | 当前胶囊折叠单张卡片；卡片分页、边缘抽屉不等于 DeskBox 的跨卡片分组/胶囊编排。 | 单独设计窗口组状态、组合/拆组、布局持久化和多屏迁移；保留已有 card ID/config 兼容。 |
| 完整 Shell 右键菜单/属性等入口 | Mochi 提供自有上下文菜单和部分 Shell 操作，没有等价于完整 Explorer Shell extension 的菜单、属性页与全部动词。 | 优先按需增加逐项 Shell verbs/属性对话框；把 COM 菜单扩展及故障隔离留作独立工程。 |
| DeskBox 式统一搜索 | Search 卡片目前只有工作区全文与 Everything 文件路径；缺少应用、系统设置、Todo、Quick Capture 的类别聚合，以及对应查询语法、历史/收藏和批量结果操作。 | 在现有搜索卡片增加可插拔来源和明确的结果类型，复用 Mochi workspace search/To-do/捕获 API；Everything 只作为可选文件索引来源。 |
| 天气/媒体的高级体验 | Weather 当前是 Open-Meteo 单源；Music 暴露基本传输控制，不等同 DeskBox 的天气 provider/皮肤与音量、循环/随机等控制布局。 | 按明确需求决定是否加替代天气源、皮肤及 GSMTC 能力探测；来源不可用时保留当前错误和缓存状态。 |
| QuickLook 条件性预览 | 仅当用户已运行 QuickLook 才能请求预览；Mochi 不安装或启动它。 | 保持可选外部依赖，或使用 Mochi 自有预览器补齐常见格式，避免静默下载第三方程序。 |

因此，“在 Mochi 卡片里浏览并管理用户映射目录”的主要路径已有实现；“直接在 Windows Explorer 桌面上完成 DeskBox 整体工作流”仍被桌面/Explorer 集成差异阻断。组件层面的差距另见上表，不应把两者混为 Folder 缺少文件操作。

## 外部依赖和系统边界

- **Everything**：文件搜索依赖用户自行安装并运行的 Everything；协议不可用时 Mochi 可同时保留工作区结果并显示来源错误。
- **Open-Meteo**：天气需要网络；没有配置默认城市或本机定位推断。
- **Windows GSMTC**：音乐卡片需要可用媒体会话；控制仅作用于用户选择或系统当前会话。
- **Windows Shell**：文件拖放、复制/移动和回收站依赖 Windows Shell/Win32 能力；QuickLook 预览需用户已运行相应程序。

## DeskBox 扫描来源

均固定在提交 `fdb5d45a5f83005c5d7a765afb2d83648fdd97ad`：

- [文件格子与映射文件夹](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/docs/articles/01-file-widgets.md)
- [桌面整理与自动整理](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/docs/articles/02-desktop-organization.md)
- [拖放与叠放交互契约](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/docs/architecture/%5B重要勿删%5Dfile_drag_stack_contract.md)
- [Everything 文件搜索实现](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/EverythingSearchService.cs)
- [天气服务](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/WeatherService.cs)
- [音乐会话服务](https://github.com/Tianyu199509/DeskBox/blob/fdb5d45a5f83005c5d7a765afb2d83648fdd97ad/src/DeskBox/Services/MusicSessionService.cs)
