// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
const html = readFileSync(resolve('public/options.html'), 'utf8')
const sendMessage = vi.fn()
beforeEach(() => {
  vi.resetModules()
  document.documentElement.innerHTML = html
  vi.stubGlobal('chrome', { runtime: { id: 'a'.repeat(32), sendMessage } })
})
afterEach(() => vi.unstubAllGlobals())
it('连接后仅展示状态和存储设置，不要求填写 ID', async () => {
  sendMessage.mockImplementation(async ({ op }) => ({ ok: true, result: op === 'context'
    ? { name: '工作区', workspace: 'token', libraries: [], defaults: { directory: '知识库/收藏' } } : {} }))
  await import('../src/options')
  await vi.waitFor(() => expect(document.querySelector('#connection-status')?.textContent).toBe('已连接墨池'))
  await vi.waitFor(() => expect(document.querySelector<HTMLElement>('#storage')?.hidden).toBe(false))
  expect(document.querySelector('#extension-id')).toBeNull()
  expect(document.querySelector('a')?.href).toBe(`mochi-clipper://connect?extension=${'a'.repeat(32)}`)
  document.querySelector<HTMLButtonElement>('#disconnect')!.click()
  await vi.waitFor(() => expect(document.querySelector('#message')?.textContent).toContain('已解除'))
  expect(document.querySelector<HTMLElement>('#storage')?.hidden).toBe(true)
})
it('没有工作区时仍显示已连接，并提示打开工作区', async () => {
  sendMessage.mockImplementation(async ({ op }) => op === 'context'
    ? { ok: false, error: '请在墨池中打开工作区，然后重试' } : { ok: true, result: {} })
  await import('../src/options')
  await vi.waitFor(() => expect(document.querySelector('#message')?.textContent).toContain('打开工作区'))
  expect(document.querySelector('#connection-status')?.textContent).toBe('已连接墨池')
  expect(document.querySelector<HTMLElement>('#storage')?.hidden).toBe(true)
})
