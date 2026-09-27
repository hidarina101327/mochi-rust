export function pairingUrl(extensionId: string): string {
  if (!/^[a-p]{32}$/.test(extensionId)) throw new Error('无效的浏览器扩展')
  return `mochi-clipper://connect?extension=${extensionId}`
}

export async function waitForConnection(probe: () => Promise<unknown>, signal: AbortSignal) {
  const deadline = Date.now() + 120000
  while (!signal.aborted && Date.now() < deadline) {
    try {
      await probe()
      signal.throwIfAborted()
      return
    } catch {
      signal.throwIfAborted()
    }
    await new Promise<void>(resolve => {
      const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve() }
      const timer = setTimeout(finish, 1500)
      signal.addEventListener('abort', finish, { once: true })
    })
  }
  signal.throwIfAborted()
  throw new Error('连接未完成。请先安装并打开新版墨池，再点击“连接墨池”。')
}
