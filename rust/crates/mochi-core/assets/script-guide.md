# 脚本批处理：先判断，再批量，最后验证

- 简单的一次查询或编辑使用现有专用工具。多维表格、日程、块文档、设置等应用数据必须使用相应工具，不要用脚本绕过数据校验直接修改 `.mcb` / `.mc` / 内部配置。已有批量工具优先于脚本。
- 重复计算、文本/JSON/CSV 清洗、文件清单和统计适合脚本；不要逐条调用上百次工具。先用少量样本确认编码、格式和目标，再选算法（流式读取、集合/字典索引、一次批量输出），避免把全量大文件灌入模型上下文。
- 首次使用先调用 script_environment；优先内置 Python（标准库 json/csv/pathlib/re/hashlib/sqlite3/zipfile 等），Windows 管理用 PowerShell，CMD 仅用于简单兼容命令。Python 不依赖用户环境，不带 pip/pandas；缺失环境时报告错误，不擅自安装或退回系统 Python。
- script_run 提交完整多行 code，不要转义成 shell 命令或 base64。提供 summary（目标、明确范围、预期影响）、intent（inspect/preview/apply）和 input（JSON 参数）。cwd 默认工作区；需要的文件路径必须来自用户任务或已读取的清单，不猜测路径。
- JSON 参数存放在环境变量 MOCHI_SCRIPT_INPUT 指向的 UTF-8 文件；MOCHI_WORKSPACE 为工作区根；MOCHI_SCRIPT_INTENT 为声明的意图。Python 示例：`data = json.loads(pathlib.Path(os.environ['MOCHI_SCRIPT_INPUT']).read_text(encoding='utf-8'))`。勿将不可信文件内容当代码或 shell 参数执行。
- 批量修改先单独运行 preview：只读取、输出 total/matched/skipped/failed、少量 before/after 样例、文件哈希和拟修改清单；检查异常后再申请 apply。第二阶段重新校验哈希，遇冲突停止；只处理预览确认的显式目标。备份原件、原子替换、可重试且幂等；删除、大范围移动、网络上传、安装依赖须单独说明并获用户授权。不要递归扫描整个盘或用户目录。
- 每次脚本都需要执行命令权限和逐次审批，不继承 shell 白名单。审批前只是提案，不代表已经运行；等真实结果再判断下一步，不重复提交同一待批脚本。
- intent 只是声明，preview 不提供强制只读；Python 的 -I 只是依赖隔离。脚本具有当前用户的系统/文件/网络权限，不是安全沙箱；不得把审批界面或启动目录限制描述成安全沙箱。
- 默认超时 120 秒，最多 300 秒，支持取消和进程树终止，Windows 进程树提交内存上限 512 MiB，输出限制 20 万字符；主动只输出结构化摘要与少量样例。检查 exitCode、ok、stderr 和结果数量，失败不宣称成功、不盲目重跑可能已部分写入的脚本。大任务流式读取、按可核验的批次执行。使用用户数据前先保存界面中未保存的文档。

## Python 只读统计示例

```python
import csv, heapq, json, os, pathlib
data = json.loads(pathlib.Path(os.environ['MOCHI_SCRIPT_INPUT']).read_text(encoding='utf-8'))
root = pathlib.Path(os.environ['MOCHI_WORKSPACE']).resolve()
source = (root / data['path']).resolve()
if not source.is_relative_to(root) or not source.is_file():
    raise ValueError('输入必须是工作区内明确指定的文件')
counts, samples, total = {}, [], 0
with source.open(encoding='utf-8-sig', newline='') as stream:
    for row in csv.DictReader(stream):
        total += 1
        key = row.get(data['column'], '')
        counts[key] = counts.get(key, 0) + 1
        if len(samples) < 3:
            samples.append(row)
top = heapq.nlargest(20, counts.items(), key=lambda pair: pair[1])
print(json.dumps({'total': total, 'distinct': len(counts), 'top20': top, 'samples': samples}, ensure_ascii=False))
```
