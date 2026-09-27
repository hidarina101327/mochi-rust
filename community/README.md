# Mochi 官方市场发布目录

此目录是 `hidarina101327/mochi-community` 空仓库的启动文件。将本目录内容（包括隐藏的 `.github`）放到社区仓库根目录，然后添加真实资源。此工作流在社区仓库运行，不会发布 Mochi 客户端安装包。

```text
plugins/github/marketplace.json
plugins/github/manifest.json
plugins/github/ui.json
plugins/slack/...
plugins/postgres/...
workflows/code-review/marketplace.json
workflows/code-review/workflow.json
workflows/research/...
templates/coding-agent/marketplace.json
templates/coding-agent/编码助手.md
```

每个目录都必须有 `marketplace.json`，例如 `templates/coding-agent/marketplace.json`：

```json
{
  "id": "coding-agent",
  "kind": "template",
  "category": "agent",
  "title": "编码助手",
  "version": "1.0.0",
  "summary": "从需求拆解到代码审查的助手模版。",
  "description": "这里填写完整使用说明、适用场景和所需配置。",
  "image": "cover.png",
  "official": true,
  "author": "Mochi"
}
```

`image` 可省略，支持包内 PNG/JPEG/WebP 或 HTTPS URL。本地封面作为独立 Release 图片上传。`official` 默认 `false`，由维护者审核后填写，不根据仓库位置自动推定官方身份。

父目录与 `kind` 的对应关系：`documents/document`、`bases/base`、`canvases/canvas`、`workflows/workflow`、`templates/template`、`agents/agent`、`knowledge-bases/knowledge-base`、`plugins/plugin`。`category` 使用同一组单数值，可以与安装类型不同，例如 Agent 模版使用 `kind: template`、`category: agent`。插件分类在 UI 中预留，真实原生插件包准备好后可以发布。

本地构建：

```sh
python tools/package-release.py --root . --output dist --tag market-v1.0.0
```

输出每个目录独立的 `github-plugin.zip`、`slack-plugin.zip`、`postgres-plugin.zip`、`code-review-workflow.zip`、`research-workflow.zip`、`coding-agent-template.zip`，以及 `marketplace.json` 汇总索引。ZIP 根目录直接包含包内容，不包裹整个仓库。构建使用固定文件顺序和 ZIP 时间戳，索引包含每个 ZIP 的 SHA-256；输出目录必须为空。

推送 `market-v*` 标签后，工作流先创建草稿 Release，上传所有资源，再公开为最新 Release。客户端读取最新正式 Release，因此每次发布应包含当前全部在售资源目录。用户只下载所选 ZIP，无须 Git 或 clone 仓库。

```sh
mochi-community install plugin github
mochi-community install workflow code-review
mochi-community install template coding-agent
mochi-community install template coding-agent --workspace /path/to/workspace
```

命令行程序随 Mochi 原生安装包提供；未加入 PATH 时使用安装目录下 `mochi-community.exe` 的完整路径。默认目标是当前目录。插件通过原生插件安装器安装；工作流导入工作区工作流库，定时任务初始关闭；模版放入 `.mochi/templates/<id>/`。安装不会执行脚本。已存在的插件和模版不会自动覆盖；其他资源类别可先下载后按需导入。
