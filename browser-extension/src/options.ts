import { native, directoryTree } from './ui'
import { pairingUrl, waitForConnection } from './pairing'
import type { Context } from './types'
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T
let context: Context | undefined, libraryId: string | undefined, folder = ''
let waiting: AbortController | undefined
let generation = 0
const connect = $<HTMLAnchorElement>('connect')
connect.href = pairingUrl(chrome.runtime.id)

async function showConnection(current: number) {
  await native('ping')
  if (current !== generation) return
  $('connection-status').textContent = '已连接墨池'
  connect.textContent = '重新连接'
  $('disconnect').hidden = false
  $('message').textContent = ''
  context = undefined
  libraryId = undefined
  folder = ''
  $('storage').hidden = true
  $('target').textContent = ''
  $<HTMLButtonElement>('set-default').disabled = true
  try {
    const next = await native<Context>('context')
    if (current !== generation) return
    context = next
    $('message').textContent = next.name
    $('storage').hidden = false
    $('default-label').textContent = next.defaults.directory
    directoryTree($('tree'), next, (id, path, label) => {
      libraryId = id
      folder = path
      $('target').textContent = label
      $<HTMLButtonElement>('set-default').disabled = false
    })
  } catch (error) {
    if (current === generation) $('message').textContent = errorText(error)
  }
}

// 在原点击事件中完成导航，外部应用提示由浏览器负责显示。
connect.onclick = event => {
  if (waiting) { event.preventDefault(); return }
  const current = ++generation
  waiting = new AbortController()
  const signal = waiting.signal
  $('cancel').hidden = false
  $('storage').hidden = true
  $('connection-status').textContent = '等待连接'
  $('disconnect').hidden = true
  $('message').textContent = '请允许浏览器打开墨池，并在客户端确认连接。'
  void waitForConnection(() => native('ping'), signal)
    .then(() => showConnection(current))
    .catch(error => {
      if (current !== generation) return
      $('connection-status').textContent = '尚未连接'
      $('message').textContent = errorText(error)
    })
    .finally(() => {
      if (current !== generation) return
      waiting = undefined
      $('cancel').hidden = true
    })
}
$('cancel').onclick = () => {
  waiting?.abort()
  waiting = undefined
  generation++
  $('cancel').hidden = true
  $('connection-status').textContent = '连接墨池'
  $('message').textContent = '已停止等待，可重新连接。'
}
$('disconnect').onclick = async () => {
  const button = $<HTMLButtonElement>('disconnect')
  button.disabled = true
  try {
    await native('disconnect')
    generation++
    context = undefined
    $('storage').hidden = true
    button.hidden = true
    $('connection-status').textContent = '连接墨池'
    connect.textContent = '连接墨池'
    $('message').textContent = '已解除当前扩展的授权。'
  } catch (error) {
    $('message').textContent = errorText(error)
  } finally { button.disabled = false }
}

$('set-default').onclick = async () => {
  if (!context || !libraryId) return
  const current = generation
  const button = $<HTMLButtonElement>('set-default')
  button.disabled = true
  try {
    const result = await native<{ directory: string }>('defaults', {
      workspace: context.workspace, libraryId, folder,
    })
    if (current !== generation) return
    $('default-label').textContent = result.directory
    $('message').textContent = '默认位置已保存。'
  } catch (error) {
    if (current === generation) $('message').textContent = errorText(error)
  } finally {
    if (current === generation) button.disabled = false
  }
}
function errorText(error: unknown) { return error instanceof Error ? error.message : String(error) }
void showConnection(generation).catch(() => {
  if (!generation) $('message').textContent = '首次连接前，请先安装并打开新版墨池。'
})
