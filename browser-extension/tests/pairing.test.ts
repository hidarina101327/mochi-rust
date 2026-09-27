import { afterEach, describe, expect, it, vi } from 'vitest'
import { pairingUrl, waitForConnection } from '../src/pairing'
import { nativeError } from '../src/protocol'

afterEach(() => vi.useRealTimers())
describe('客户端授权连接', () => {
  it('自动携带当前扩展身份，拒绝无效身份', () => {
    expect(pairingUrl('a'.repeat(32))).toBe(`mochi-clipper://connect?extension=${'a'.repeat(32)}`)
    expect(() => pairingUrl('a&command=run')).toThrow()
  })
  it('宿主注册后自动连接', async () => {
    vi.useFakeTimers()
    const probe = vi.fn().mockRejectedValueOnce(new Error('not registered')).mockResolvedValue({})
    const result = waitForConnection(probe, new AbortController().signal)
    await vi.advanceTimersByTimeAsync(1500)
    await result
    expect(probe).toHaveBeenCalledTimes(2)
  })
  it('取消等待后不会继续连接', async () => {
    vi.useFakeTimers()
    const controller = new AbortController()
    const probe = vi.fn().mockRejectedValue(new Error('not registered'))
    const result = waitForConnection(probe, controller.signal)
    const rejected = expect(result).rejects.toThrow()
    await vi.advanceTimersByTimeAsync(0)
    controller.abort()
    await rejected
    await vi.advanceTimersByTimeAsync(150000)
    expect(probe).toHaveBeenCalledTimes(1)
  })
  it('未安装或未批准时结束等待并提供操作提示', async () => {
    vi.useFakeTimers()
    const result = waitForConnection(vi.fn().mockRejectedValue(new Error('not registered')), new AbortController().signal)
    const rejected = expect(result).rejects.toThrow('安装并打开新版墨池')
    await vi.advanceTimersByTimeAsync(120000)
    await rejected
  })
  it('宿主缺失和未授权错误显示可操作的中文信息', () => {
    expect(nativeError('Specified native messaging host not found. &#x20;')).toContain('连接墨池')
    expect(nativeError('Access to the specified native messaging host is forbidden.')).toContain('完成授权')
    expect(nativeError('未知错误&#x20;')).toBe('未知错误')
  })
})
