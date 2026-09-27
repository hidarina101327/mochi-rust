# {{接口名称}} · 接口文档

**负责人：** 待填写 · **更新：** {{日期}}

:::mochi-highlight color="blue" title="接口用途"
说明谁在什么场景下调用这个接口，以及成功后会发生什么。
:::

## 请求约定

下方使用 `GET /api/items` 作为示例，请替换为实际接口。

| 参数 | 位置 / 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| limit | query / integer | 否 | 每页条数，填写默认值与上限 |
| cursor | query / string | 否 | 分页位置，首次请求可省略 |

**鉴权：** 填写调用身份与所需权限，不在文档中保存真实密钥。

<!-- mochi-code-block title="请求示例" collapsed="false" -->
```http
GET /api/items?limit=20
Accept: application/json
```

<!-- mochi-code-block title="成功响应示例" collapsed="false" -->
```json
{
  "items": [{ "id": "example-1", "title": "示例条目" }],
  "nextCursor": null
}
```

## 错误处理

| 状态 / 错误码 | 触发条件 | 调用方处理 |
| --- | --- | --- |
| 400 | 参数不符合约定 | 修正参数后重试 |
| 401 | 身份校验未通过 | 完成身份校验 |
| 500 | 服务处理失败 | 记录请求标识，按约定重试 |

<details>
<summary>展开边界场景</summary>

补充空列表、末页、超大参数、重复请求与权限不足时的实际响应。

</details>

- [ ] 请求与响应示例已与实现核对
- [ ] 默认值、空值与错误约定已明确
- [ ] 调用方已完成关键场景联调
