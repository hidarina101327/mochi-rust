import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { VERSION, HOST, MAX_TOTAL, MAX_IMAGE, encode, decode, readingHtml } from '../src/protocol'
const fixture = JSON.parse(
  readFileSync(path.resolve('../shared/web-clipper-protocol.json'), 'utf8')
)
describe('与 Rust 共用的协议', () => {
  it('版本、宿主和大小限制一致', () => {
    expect(VERSION).toBe(fixture.version)
    expect(HOST).toBe(fixture.host)
    expect(MAX_TOTAL).toBe(fixture.limits.totalBytes)
    expect(MAX_IMAGE).toBe(fixture.limits.imageBytes)
  })
  it('二进制分块不损坏 PDF / PNG 字节', () => {
    const bytes = Uint8Array.from({ length: 196608 }, (_, i) => i % 256)
    expect(decode(encode(bytes))).toEqual(bytes)
  })
  it('阅读版转义元信息并禁止执行脚本', () => {
    const html = readingHtml('<标题>', 'https://example.com/?a="b"', '<p>正文</p>', 0)
    expect(html).toContain('&lt;标题&gt;')
    expect(html).toContain("default-src 'none'")
    expect(html).toContain('<p>正文</p>')
  })
})
