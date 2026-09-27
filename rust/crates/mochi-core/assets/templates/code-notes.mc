# {{代码主题}} · 代码笔记

**语言 / 环境：** 待填写 · **记录日期：** {{日期}}

:::mochi-highlight color="blue" title="这段代码解决什么"
先描述输入、预期输出和适用条件，再记录实现。
:::

## 最小示例

以下为“保持顺序去重”的示例，可直接替换为自己的代码。

<!-- mochi-code-block title="保持顺序去重 · Python" collapsed="false" -->
```python
def unique_in_order(values):
    seen = set()
    result = []
    for value in values:
        if value not in seen:
            seen.add(value)
            result.append(value)
    return result

assert unique_in_order([3, 1, 3, 2]) == [3, 1, 2]
```

## 关键解释

| 设计选择 | 为什么这样写 | 使用边界 |
| --- | --- | --- |
| 用集合记录已见元素 | 加快成员查找 | 元素需要可哈希 |
| 单独保留结果列表 | 保持首次出现的顺序 | 需要额外空间 |

在集合查询平均为常数时间的假设下，时间与额外空间复杂度为：

$$
T(n) = O(n), \qquad S(n) = O(n)
$$

<details>
<summary>展开逐步推演</summary>

用一个小输入逐步记录变量变化，说明重复元素在哪一步被跳过。

</details>

## 验证清单

- [ ] 空输入与单元素输入
- [ ] 全部重复与全部不同的输入
- [ ] 不满足前提条件时的行为

**迁移到我的项目：** 哪些假设需要重新验证？
