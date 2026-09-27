// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import '../src/extract'
import { sanitize } from '../src/extract'
beforeEach(() => {
  document.title = '网页标题'
  document.body.innerHTML = ''
  window.getSelection()?.removeAllRanges()
})
describe('网页正文', () => {
  it('保留表格代码，去掉活动内容，转换相对链接', () => {
    document.body.innerHTML =
      '<article><h1>文章标题</h1><p>' +
      '这是一段用于验证剪藏的中文正文。'.repeat(50) +
      '</p><a href="/reference">来源</a><table><thead><tr><th>列</th></tr></thead><tbody><tr><td>内容</td></tr></tbody></table><pre><code>const x = 1;</code></pre><img src="https://example.com/a.png" onerror="alert(1)"></article>'
    const article = globalThis.__mochiExtract!('article')
    expect(article.markdown).toContain('| 列 |')
    expect(article.markdown).toContain('const x = 1;')
    expect(article.html).not.toContain('onerror')
    expect(article.images[0].url).toBe('https://example.com/a.png')
    expect(article.markdown).toContain('mochi-asset-0-end')
  })
  it('选区只保存选中内容，空选区报错', () => {
    document.body.innerHTML = '<p>不选择</p><p id="selected">需要保存的文字</p>'
    expect(() => globalThis.__mochiExtract!('selection')).toThrow()
    const range = document.createRange()
    range.selectNodeContents(document.getElementById('selected')!)
    window.getSelection()!.addRange(range)
    const clip = globalThis.__mochiExtract!('selection')
    expect(clip.markdown).toBe('需要保存的文字')
    expect(clip.html).not.toContain('不选择')
  })
  it('移除脚本、表单和 javascript 链接', () => {
    const result = sanitize(
      '<script>alert(1)</script><form><input value="secret"></form><a href="javascript:alert(1)" onclick="evil()">link</a>'
    )
    expect(result).not.toMatch(/script|input|onclick|javascript/i)
  })
})
