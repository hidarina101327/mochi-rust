# 墨池网页剪藏

独立发布的 Chrome / Edge Manifest V3 扩展，仅接入 Windows Rust 版墨池。

## 构建与打包

```powershell
cd browser-extension
npm ci
npm test
npm run package
```

开发时在浏览器扩展管理页开启开发者模式，加载 `dist`。`artifacts` 中的 Chrome / Edge ZIP 可分别上传市场；ZIP 根目录包含 manifest，不包含 node_modules、源码或 Rust 程序。不要用浏览器“打包扩展”生成的 CRX 上传市场。

Rust 构建：在项目根目录运行 `cargo build --manifest-path rust/Cargo.toml -p mochi-app -p mochi-core --bin mochi-app --bin mochi-clipper-host`，确保两个 EXE 在同一目录。正式安装包通过 `rust/tools/package-native.ps1` 构建。

## 首次绑定

1. 安装或运行新版 Rust 版墨池，打开一个工作区。便携版首次需运行一次客户端。
2. 打开扩展设置，点击「连接墨池」。
3. 在浏览器中允许打开墨池，再在客户端确认连接。设置页会自动显示连接结果。
4. 其他浏览器重复上述步骤即可，已有连接会保留。无需复制或填写扩展 ID。

开发者加载、Chrome 市场和 Edge 市场的身份由授权流程自动识别。扩展设置中的「断开连接」解除当前扩展身份；相同扩展 ID 的浏览器共享授权。墨池设置中关闭网页剪藏会停用全部连接，再次连接并确认会重新启用。升级或移动便携版后运行新客户端即可更新注册路径。

首次连接通过 `mochi-clipper://connect` 唤起客户端，由客户端明确确认后追加授权身份，再注册 Chrome / Edge Native Messaging 宿主。后续数据传输仍走命名管道。连接页面超时通常表示未安装新版客户端、未允许打开客户端或未确认授权；先运行新版墨池再重试。宿主缺失时会提示重新连接，不再直接展示英文报错。

## 保存

- 快速存储：进入收件箱，完整正文和附件独立保存，可打开、编辑摘要 / 标题、归档和删除。
- 默认知识库：首次创建「知识库/浏览器收藏」。在墨池或扩展设置中修改默认位置。
- 自定义存储：按需展开知识库 / 文件夹树，本次选择不改变默认位置。

每次剪藏是一组独立文件：`标题--剪藏ID/document.md|html|pdf` 与 `assets/`。移动或归档整组文件不会损坏图片链接。同一任务重试不重复保存；再次主动收藏创建新条目。

全文为正文提取；HTML 为阅读版，PDF 为原网页的打印布局，可能受网站打印样式影响。选区 PDF 保留可复制的文字样式；截图 PDF 为图片。首版不做长截图、OCR、受保护页面或跨域 iframe 正文提取。

离线图片仅在得到对应站点权限后下载，失败时保留原链接并提示。PDF 首次需要可选 debugger 权限，浏览器可能显示调试提示；开发者工具占用、策略限制或用户取消会明确报错，不降级生成其他格式。

## 本地数据和故障处理

墨池未运行时，本地宿主启动同目录的墨池程序。没有工作区时，先在墨池中选择工作区，再重试。保存期间切换工作区会拒绝后续请求，避免写错位置；切回原工作区后可重试。

关闭扩展弹窗不会取消已开始的后台任务。失败内容保存在浏览器 IndexedDB，成功后清除正文和附件。上传中浏览器重启会恢复；内容提取阶段被终止时，需要回到原页面重试。移除失败记录会删除浏览器暂存内容。

单次剪藏最多 100 MiB，单张图片最多 20 MiB。接收端暂存和成功回执位于工作区 `.mochi/web-clipper/`，成功回执用于幂等重试。未提交的上传可在确认无需重试后由用户清理对应 ID 目录。

协议样例在仓库 `shared/web-clipper-protocol.json`。Windows 使用当前用户 ACL 命名管道，不监听 HTTP 端口。


## 隐私说明


墨池网页剪藏只在用户操作扩展时读取当前网页、所选内容或网页截图。采集标题、来源 URL、可获得的作者、剪藏时间、正文及用户选择的图片。

内容通过浏览器 Native Messaging 和本机命名管道传给本地 Rust 版墨池，并保存到用户工作区。扩展不提供云端上传，不发送分析、广告或遥测数据。

下载离线图片时会向原图片站点发起请求，可能携带浏览器允许的登录凭据。图片站点有自己的隐私政策。正文转换及 PDF 生成在本机完成。

尚未保存成功的任务保存在浏览器本地 IndexedDB，可从最近剪藏列表删除。成功后移除浏览器中的正文和附件，保留简短结果记录；删除结果记录不会删除已经保存的墨池文件。关闭或移除扩展可以终止后续访问。

权限用途：activeTab / scripting 用于用户指定页面的提取与框选；storage 用于偏好及任务状态；nativeMessaging 用于本机墨池通信；alarms 用于中断上传恢复。可选站点权限用于离线图片；可选 debugger 权限仅用于浏览器打印 PDF，用完立即解除调试连接。

首次启用需用户在墨池中绑定扩展 ID。可随时在墨池设置中解除绑定或关闭网页剪藏。


## 第三方声明


墨池扩展沿用项目 MIT 许可证。图标来自本仓库的墨池图标。

运行时组件：

- Mozilla Readability — Apache-2.0，https://github.com/mozilla/readability
- DOMPurify — Apache-2.0 OR MPL-2.0，https://github.com/cure53/DOMPurify
- Turndown — MIT，https://github.com/mixmark-io/turndown
- turndown-plugin-gfm — MIT，https://github.com/mixmark-io/turndown-plugin-gfm
- pdf-lib — MIT，https://github.com/Hopding/pdf-lib
- @pdf-lib/standard-fonts、@pdf-lib/upng、pako、tslib — 各组件许可证随发行包保留。

完整许可证文本由构建脚本收集到 `licenses/` 并随扩展 ZIP 发布。未从 CDN 或远程地址加载任何可执行代码。
