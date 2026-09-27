# {{功能名称}} · 技术方案

**作者：** 待填写 · **更新：** {{日期}} · **状态：** 待评审

:::mochi-highlight color="blue" title="设计目标"
解决什么具体问题？性能、兼容性、可维护性等约束中，哪些是必须满足的？
:::

## 方案概览

**数据流：** 输入 → 校验 → 处理 → 持久化 → 反馈。

| 组件 | 职责 | 输入 / 输出 |
| --- | --- | --- |
| 入口 | 接收请求与校验 | 说明边界 |
| 业务层 | 核心规则与状态变化 | 说明失败情况 |
| 存储层 | 持久化与一致性 | 说明读写方式 |

## 关键接口

下面是接口草图，替换为本项目的类型与约定。

<!-- mochi-code-block title="接口草图 · TypeScript" collapsed="false" -->
```typescript
type Result<T> =
  | { ok: true; value: T }
  | { ok: false; error: string };

interface Service<Input, Output> {
  execute(input: Input): Promise<Result<Output>>;
}
```

## 取舍

| 方案 | 优点 | 代价 | 结论 |
| --- | --- | --- | --- |
| 推荐方案 | 满足哪些关键条件 | 增加什么复杂度 | 为什么选择 |
| 备选方案 | 哪些场景更适合 | 放弃的原因 | 何时重新考虑 |

:::mochi-highlight color="yellow" title="失败与恢复"
超时、重复请求、部分写入或版本不兼容时，如何检测、恢复与避免扩大影响？
:::

- [ ] 定义关键场景的验证方式
- [ ] 确认迁移与回退步骤
- [ ] 明确上线后需要观察的信号
