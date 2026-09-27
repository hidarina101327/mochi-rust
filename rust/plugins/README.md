# 墨池原生插件 v2

墨池的第三方插件不依赖 Electron，也不会加载 HTML、JavaScript 或 iframe。插件是 ZIP 包；宿主先校验清单和权限，再用 Rust 绘制声明式界面。

## 最小插件

```json
// manifest.json
{
  "manifestVersion": 2,
  "id": "com.example.reading-log",
  "name": "阅读记录",
  "version": "1.0.0",
  "permissions": ["plugin-storage", "workspace-read"],
  "contributions": {
    "navigation": true,
    "commands": ["新建阅读记录"],
    "ui": "ui.json",
    "slots": [
      { "slot": "navigation", "title": "阅读记录", "icon": "BookOpen" },
      { "slot": "sidebar", "title": "阅读进度" },
      { "slot": "right-sidebar", "title": "阅读详情", "ui": "inspector.json" },
      { "slot": "editor-toolbar", "title": "插入阅读引用" }
    ]
  }
}
```

`id` 仅可包含 ASCII 字母、数字、`.`、`_` 和 `-`。权限取最小集合：`workspace-read`、`workspace-write`、`plugin-storage`、`network`、`shell-launch`、`ai-read`、`ai-write`。

`slots` 是受控宿主插槽：`navigation` 会创建全局导航入口；`sidebar` 保留文件栏底部区域；`right-sidebar` 出现在右侧栏的插件标签中；`editor-toolbar` 为后续编辑器命令按钮预留。插件声明位置和 UI，宿主负责布局、生命周期、事件分发与权限，避免第三方代码破坏整个窗口。

```json
// ui.json
{
  "kind": "dashboard",
  "title": "阅读记录",
  "actions": ["新增记录"],
  "sections": [
    { "title": "本周", "cards": ["已读 3 本", "阅读 240 分钟"] }
  ]
}
```

## 创作、打包与导入

1. 在「Agent 配置」中使用“插件创作助手”生成插件工程。
2. 在「设置 → 第三方库」点击“打包插件工程”，选择含 `manifest.json` 的文件夹。
3. 点击“导入原生 ZIP”。插件会安装到工作区 `.mochi/plugins/<id>`，可在同页启用、禁用或移除。

`english-lab-v2.zip` 与 `quick-navigation-v2.zip` 是旧 Electron 插件的原生迁移示例。它们的旧 HTML/JS 运行时不被复用；可复用的是功能模型、数据结构、命令和 AI 工具定义，界面与宿主能力边界由 v2 重新声明。
