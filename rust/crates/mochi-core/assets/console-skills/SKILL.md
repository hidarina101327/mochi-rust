---
name: 控制台能力契约
description: 墨池控制台工具的按需操作参考；由各模块 Skill 加载
---

先用 skill_list 查找模块，再用 skill_load 加载该模块。
出现“控制台扩展”提示时，继续调用 skill_load，id 保持不变，reference 使用 console-api.md。
运行时只返回该 Skill 所含工具的参数契约，不将所有模块的参数注入通用助手。
参数契约随安装包发布；工作区中的自定义 Skill 正文保留不变。
