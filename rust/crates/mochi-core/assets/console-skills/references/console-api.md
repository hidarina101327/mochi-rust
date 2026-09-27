所有工具使用 action 指定操作。id、path、revision、data、operations 位于参数顶层。
先读后写；只复用真实返回的 ID、路径和 revision。批次最多 64 项。
读结果含 {revision,value} 时，写操作传同一 revision。版本冲突后重新读取并合并。
工具仍遵循动作权限、目录权限和审批模式；拒绝后不要改用脚本绕过。返回 pendingConsoleAction 时尚未执行，等待用户在会话卡片中批准；不要重复提交。
后台操作返回 {jobId,status:running}。调用同工具 {action:status,id:jobId} 查询，直到 completed 或 failed；running 不能报告已完成。遵循 pollAfterMs，避免密集轮询。

## console_state
action:get，无其他参数。返回工作区、当前模块、标签索引/路径/脏状态、活动标签、分屏、设置、弹窗和 AI 面板状态。

## console_navigate
action:open，data.module 为 home/editor/inbox/knowledge/favorites/recent/templates/schedule/ai/agents/quick_note/workflows/desktop_cards/marketplace/settings/notifications/english；settings 可传 data.section。

## console_tabs
open/focus/close/split/locate：path 为文档路径。close 拒绝未保存标签。
split 可加 data.ratio（0.2–0.8），仅支持文本编辑器。unsplit 关闭分屏；focus_pane 传 data.right 布尔值。
locate 传 data.line（从 1 开始）。PDF 用 annotations_manage locate。
示例：{"action":"locate","path":"知识库/设计.md","data":{"line":25}}。

## console_workspace
get 返回当前状态。switch 传 path 为目标工作区绝对路径，拒绝未保存标签；切换会结束当前工作区的 Agent 运行，需在目标工作区开启下一次请求。

## inbox_manage
list 可传 data.status（all/inbox/archived）。create 传 data.content，可选 source。
update 传 id 和 data.content/status（inbox/archived）；delete 传 id。
archive 传 id 和 path（已有目标目录），生成笔记并归档；to_task 传 id，创建任务。
batch 的 operations 每项是独立调用参数 {action,id?,path?,data?}；支持 create/update/delete/archive/to_task。
批处理返回 atomic:false 和逐项结果，部分失败时只重试失败项，不重复创建已成功条目。

## favorites_manage
list 返回路径列表及 revision。add/remove 传 path；重复添加或移除是幂等操作。
reorder 传 revision、data.paths，必须包含原列表全部路径，每项恰好一次。

## recent_manage
list 查询最近修改的文档。remove 传 path，clear 隐藏全部当前记录，均不删除原文件。
文档再次修改后会重新出现；此模块遵循控制台最近文档的修改时间语义。

## unread_manage
list 返回路径到版本 token 的映射。mark 传 data.paths 字符串数组。
read 传 path、data.token（list 返回的原 token）；不将读取后新增的修改误标为已读。

## templates_manage
list 返回分组、模板路径与 revision；get 传 path 获取正文。
create 传 data.name/content，可选 group（默认未分组）。update 传 path/revision/data.content；delete 传 path。
instantiate 传模板 path、data.parent（已有目标目录），返回新文档路径。
group_create/group_delete 传 data.name；group_rename 传 data.name/newName；move 传模板 path、data.group。
分组删除遵循模板服务对非空分组的限制。

## base_manage
get 传 .mcb path，返回完整结构及 revision。batch 传 path/revision/operations。
每项使用 op：table_create{name}；table_delete{id}；table_rename{tableId,name}；record_update{tableId,id,values}；record_delete{tableId,id}；field_save{tableId,field}；field_delete{tableId,id}；view_save{tableId,view}；view_delete{tableId,id}。
values 是字段 ID 到新值的映射，只修改提供的字段。field/view 为完整对象，先复用 get 的结构；新建需稳定唯一 ID。
field 至少含 id/name/type；view 含 id/name/type/sorts/filters；保留已有扩展属性。
字段删除会移除对应单元格及视图引用。表、字段和视图必须保留格式所需的最小结构；可在同一批次中先新增替代项再删除。
整批在内存中校验，通过才写文件；任何一项无效都不写入。存在未保存编辑时拒绝覆盖。
创建记录继续使用 base_create_records。

## canvas_manage
get 传 .mcanvas path，返回画布内容及 revision。batch 传 path/revision/operations。
对象操作：{kind:"cards"|"texts"|"strokes",op:"delete"|"update",id,changes?}。
changes 是已存在属性到新值的映射，不能修改 id。卡片可改 x/y/width/height/target/color（color 为 0–16777215 的 RGB 整数，null 恢复主题默认色）；文字和笔迹样式按 get 中的字段修改。
视口操作：{kind:"viewport",value:{x:0,y:0,zoom:1}}，zoom 范围 0.01–4。
卡片 target 使用现有 Mochi 对象引用格式，修改引用前查询真实目标。纯删除也必须携带 revision。所有操作先校验再整体应用，并接入画布撤销记录。
新增对象仍使用 canvas_add_cards/canvas_draw。

## comments_manage
list 传文档 path，返回评论及 revision。
create 传 path/revision/data.content，可选 author/anchor；anchor 按读取结果的文档锚点结构提供。
reply 另传父评论 id。update 传评论 id、data.content；resolve/reopen 传 id；delete 删除该评论及其全部回复。
每次修改都传最近 list 的 revision。返回新列表；resolved 状态写入评论侧文件。

## annotations_manage
list 传 PDF path，返回批注及 revision。
create 传 path/revision，data 为 {type:"rect"|"circle"|"text",page:1,x:0.1,y:0.1,width:0.2,height:0.1,color:"#ffcc00",text?:"说明"}；坐标和宽高为页面归一化值 0–1，page 从 1 开始；ID 和时间自动生成。
update 传 id/path/revision，data 提供要改的属性；delete 传 id/path/revision。
locate 传 PDF path，可用 id 定位批注，或 data.page 定位页码；PDF 正在加载时需等待后重试。

## calendar_navigate
action:open，打开日程待办。可传 data.view（today/week/month/confirm/tasks/goals/wishes/routines/projects/trash）、data.date（YYYY-MM-DD）和 data.id（任务、日程、目标、愿望、重复安排或项目 id）。给 id 时选中对应记录并打开检查器；修改数据使用 agenda_batch，不用这个动作。

## desktop_cards_control
open 打开卡片管理。edit 传卡片 id，可选 data.pageId；有未保存草稿时拒绝切换。
show/hide/focus 传卡片 id。显示状态写入配置；focus 仅聚焦已显示卡片。
布局和批量配置仍使用 desktop_cards_get/update/batch，先读取真实卡片/页面 ID。

## workflow_open
action:open；可传工作流 id 打开对应编辑器。未保存的工作流编辑会阻止切换。
查询、配置、批量处理和运行继续使用 workflow_* 工具。

## ai_sessions_manage
list 返回会话索引、项目分组与 revision。get 传会话 id；create 传 data.title。
update 传 id/revision，data 可含 title/pinned/order/projectId（null 表示移出分组）。
delete 传 id/revision，不允许删除当前会话。project_save 传 revision/data.name，可传 id 更新分组和 data.order；project_delete 传 id/revision，保留会话并移出分组。
export 传会话 id 和尚不存在的 path，输出 Markdown。

## agent_definitions_manage
list 查询 Agent、Skill、MCP、工具定义的 ID、路径及已分配工具。
get 传定义 path 返回正文与 revision。validate 传 path/data.content，检查 frontmatter、正文和工具注册名。
save 传 path/data.content；覆盖已有文件必须带 revision。修改 tools/skills/mcps 等 frontmatter 完成能力分配；下一次请求加载变更。
delete 传 path/revision；保护通用助手及被右键菜单引用的 Agent。
定义目录契约为 Agent配置/Agents/名称.md、Skills/名称/SKILL.md、Tools/名称/TOOL.md、MCPs/名称/MCP.md 或 QuickActions/名称.md。
updates 返回随包更新的 id/revision/currentRevision/diff/upstreamDiff。向用户展示差异后，apply_update 传 id/revision/data.currentRevision，服务保留备份；预览之后文件变化则拒绝。

## english_manage
get 可传 data.section：dashboard/due/words/dictionaries/stats/history/articles/sentences；data.limit 为 1–200。
add_word 传 data.word/meaning；import_words 传 data.name/content；remove_dictionary 传词库 id。
add_article 传 data.title/content，可选 translation/difficulty/source；add_sentence 传 data.content 及同样可选项。
grade 传词条 id、data.grade（1–4：重来/困难/良好/简单）、elapsedMs；dictation 传句子 id、data.answer/elapsedMs。评分后回读 due/stats/history。

## exam_manage
get/open 传 .exam path，可选 data.block（从 0 开始），打开试卷并读取模型、答案和评分状态。
answer 传 path/data.block/questionId/answer；答案值遵循对应题型，从 get 的模型查询题目 ID。
submit 按块提交并保存成绩；reset 清除此块作答草稿与回看状态。
update 传 path/revision/data.block/data.model；存在作答草稿时拒绝替换。rev 来自 get。

## pomodoro_manage
get 查询运行状态及剩余秒数。start 开始标准 25 分钟番茄钟；pause/resume/reset 操作同一计时器。
history 可传 data.days（1–365）读取已完成的专注记录。

## plugins_manage
list 返回已安装插件 manifest/权限/路径/enabled。install 传插件 ZIP path，返回后台任务 ID。
enable/disable/uninstall 传 list 的插件 id。package 传插件工程目录 path 和 data.destination（工作区外的绝对 ZIP 路径或允许的路径），不覆盖已有文件；返回后台任务。
status 传 jobId 为 id 查询后台结果。安装使用原生 v2 校验，不执行包内安装脚本。

## marketplace_manage
list/refresh 后台读取当前市场目录。install 传市场包 id，从官方目录重新查验版本、下载地址与校验和后安装。
status 传 jobId 为 id 查询结果。只回报实际安装成功的条目；已存在包遵循安装器防覆盖限制。

## mapped_folders_manage
list 返回完整配置及 revision。add 传 revision、data.libraryPath/source/name，可选 parentPath/rules；source 为现有外部目录绝对路径，挂载位置必须在指定知识库中。
update 传 id/revision，data 可改 name/source/rules；rules 为 {include:["**"],exclude:[".git/**"]}。
delete 传 id/revision，仅删除映射配置，不删除外部文件。

## workspace_transfer
export 的 path 为尚不存在的导出 ZIP 绝对路径，必须在工作区外；先保存未保存文档。导出实际工作区文件，跳过 Git 仓库元数据，不跟随符号链接/外部映射。
inspect 的 path 为 ZIP 绝对路径，检查目录和解压体积。import 传 ZIP path、data.destination 为尚不存在的新工作区目录绝对路径；先暂存验证，再整体移动，不覆盖现有工作区。
这些操作均返回后台任务；status 传 jobId 为 id。上限 1 GiB、100000 条目。

## sync_manage
status（不带 id）后台读取 Git 分支、变更和冲突；携带 jobId 为 id 则查询任务结果。
fetch/pull/push 可传 data.remote（默认 origin），使用工作区已配置的 Git 远程。pull 要求已保存且 Git 工作区干净；不强制推送或自动覆盖本地更改。
resolve 传 data.path（真实冲突文件）和 resolution（ours/theirs/content）；content 方式另传 data.content，保存后暂存解决结果。
需要已启用的版本库、系统 Git、命令和网络权限。身份认证沿用用户配置，不接收或输出凭据。

## updates_manage
get 返回应用当前版本和界面更新状态。check 后台查询新版本；download 后台下载并验证校验和，不自动启动安装。
status 传 jobId 为 id 查询结果。install 的 id 必须是本工具成功下载返回的 jobId，验证文件未变化、保存当前内容后启动安装并退出应用；此操作会结束当前会话。

## notifications_manage
list 返回通知与未读数。read 传字符串形式的数字 id，省略 id 标记全部已读。delete 传 id；clear 清空通知。

## home_stats
action:get，可传 data.days；返回首页汇总和活动趋势，以实际统计结果回答。
