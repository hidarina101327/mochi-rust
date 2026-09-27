export const VERSION = 1
export const HOST = 'com.mochi.web_clipper'
export const MAX_TOTAL = 100 * 1024 * 1024
export const MAX_IMAGE = 20 * 1024 * 1024
export function nativeError(message: string): string {
  if (/Specified native messaging host not found|Access to the specified native messaging host is forbidden/i.test(message))
    return '尚未连接墨池，请打开扩展设置，点击“连接墨池”完成授权。'
  if (/Native host has exited|Failed to start native messaging host/i.test(message))
    return '墨池本地连接不可用，请打开新版墨池后，在扩展设置中重新连接。'
  return message.replace(/(?:&#x20;|&#32;)/gi, ' ').trim()
}
export function encode(bytes: Uint8Array): string {
  let binary = ''
  for (let i = 0; i < bytes.length; i += 8192)
    binary += String.fromCharCode(...bytes.subarray(i, i + 8192))
  return btoa(binary)
}
export function decode(data: string): Uint8Array {
  return Uint8Array.from(atob(data), c => c.charCodeAt(0))
}
export async function sha256(blob: Blob): Promise<string> {
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256', await blob.arrayBuffer()))]
    .map(b => b.toString(16).padStart(2, '0'))
    .join('')
}
export function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!
  )
}
export function readingHtml(title: string, url: string, body: string, capturedAt: number): string {
  return `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: https: http: file:; style-src 'unsafe-inline'"><title>${escapeHtml(title)}</title><style>body{font:18px/1.8 system-ui,sans-serif;max-width:820px;margin:48px auto;padding:0 24px;color:#242424}img{max-width:100%;height:auto}pre{overflow:auto;background:#f4f4f4;padding:16px}table{border-collapse:collapse}td,th{border:1px solid #ddd;padding:8px}a{color:#176858}header{border-bottom:1px solid #ddd;margin-bottom:28px;padding-bottom:20px}small{color:#666}</style></head><body><header><h1>${escapeHtml(title)}</h1><small><a href="${escapeHtml(url)}">原文</a> · ${new Date(capturedAt).toISOString()}</small></header><main>${body}</main></body></html>`
}
let nativePort: chrome.runtime.Port | undefined
let idle: ReturnType<typeof setTimeout> | undefined
const pending = new Map<
  string,
  {
    resolve: (value: unknown) => void
    reject: (error: Error) => void
    timer: ReturnType<typeof setTimeout>
  }
>()
function connection() {
  if (idle) clearTimeout(idle)
  if (nativePort) return nativePort
  const port = chrome.runtime.connectNative(HOST)
  nativePort = port
  port.onMessage.addListener(reply => {
    const call = pending.get(reply.id)
    if (!call) return
    pending.delete(reply.id)
    clearTimeout(call.timer)
    if (reply.version !== VERSION) call.reject(new Error('协议版本不兼容，请更新墨池和扩展'))
    else if (!reply.ok) call.reject(new Error(reply.error || '保存失败'))
    else call.resolve(reply.result)
    if (!pending.size)
      idle = setTimeout(() => {
        if (nativePort === port) {
          nativePort = undefined
          port.disconnect()
        }
      }, 15000)
  })
  port.onDisconnect.addListener(() => {
    const message = nativeError(chrome.runtime.lastError?.message || '墨池连接已断开，请在扩展设置中重新连接')
    if (nativePort !== port) return
    nativePort = undefined
    for (const call of pending.values()) {
      clearTimeout(call.timer)
      call.reject(new Error(message))
    }
    pending.clear()
  })
  return port
}
export function nativeRequest<T = unknown>(
  op: string,
  fields: Record<string, unknown> = {}
): Promise<T> {
  const id = crypto.randomUUID()
  return new Promise((resolve, reject) => {
    const port = connection()
    const timer = setTimeout(() => {
      pending.delete(id)
      reject(new Error('墨池响应超时；内容已保留，可重试'))
      if (!pending.size) {
        nativePort = undefined
        port.disconnect()
      }
    }, 45000)
    pending.set(id, { resolve: value => resolve(value as T), reject, timer })
    try {
      port.postMessage({ version: VERSION, id, op, ...fields })
    } catch (error) {
      pending.delete(id)
      clearTimeout(timer)
      reject(error)
    }
  })
}
