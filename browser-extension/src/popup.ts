import { rpc, native, element, directoryTree } from './ui'
import type { Context, Job, Format, Mode, Destination, SaveRequest } from './types'
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T
let context: Context | undefined,
  tab: chrome.tabs.Tab | undefined,
  libraryId: string | undefined,
  folder = ''
let imageOrigins: string[] = []
const mode = $<HTMLSelectElement>('mode'),
  format = $<HTMLSelectElement>('format'),
  message = $('message')
$('settings').onclick = () => chrome.runtime.openOptionsPage()
function help() {
  const screenshot = mode.value === 'screenshot'
  $('crop-row').hidden = !screenshot
  $('images-row').hidden = screenshot || format.value === 'pdf'
  $('format-help').textContent =
    format.value === 'pdf' && !screenshot
      ? '全文 PDF 保留网页打印排版；选区 PDF 只保存选中内容。'
      : ''
  void chrome.storage.local.set({ format: format.value })
}
mode.onchange = help
format.onchange = help
async function save(destination: Destination) {
  if (!context || !tab?.id) return
  try {
    // 只申请本页点击操作涉及的来源和图片域名权限。
    const permissions: chrome.permissions.Permissions = {}
    if (format.value === 'pdf' && mode.value !== 'screenshot')
      permissions.permissions = ['debugger']
    if (
      $<HTMLInputElement>('images').checked &&
      mode.value !== 'screenshot' &&
      format.value !== 'pdf' &&
      imageOrigins.length
    )
      permissions.origins = imageOrigins
    if (permissions.permissions?.length || permissions.origins?.length) {
      const granted = await chrome.permissions.request(permissions)
      if (!granted && permissions.permissions?.length)
        throw new Error('PDF 需要调试权限；可改选 Markdown 或 HTML')
    }
    const request: SaveRequest = {
      tabId: tab.id,
      workspace: context.workspace,
      mode: mode.value as Mode,
      format: format.value as Format,
      destination,
      libraryId,
      folder,
      downloadImages: $<HTMLInputElement>('images').checked,
      crop: $<HTMLInputElement>('crop').checked,
      title: $<HTMLInputElement>('title').value,
    }
    await rpc({ type: 'save', request })
    message.textContent = '正在保存，可以关闭此弹窗。'
    if (request.mode === 'screenshot') window.close()
    else await showJobs()
  } catch (error) {
    message.textContent = String(error)
  }
}
$('inbox').onclick = () => void save('inbox')
$('default').onclick = () => void save('default')
$('save-custom').onclick = () => void save('custom')
$('custom').onclick = () => {
  if (!context) return
  $('picker').hidden = !$('picker').hidden
  if (!$('picker').hidden)
    directoryTree($('tree'), context, (id, path, label) => {
      libraryId = id
      folder = path
      $('target').textContent = label
      $<HTMLButtonElement>('save-custom').disabled = false
    })
}
async function showJobs() {
  try {
    const records = await rpc<Job[]>({ type: 'jobs' })
    const box = $('jobs')
    box.replaceChildren()
    for (const job of records.slice(0, 8)) {
      const row = element('div', undefined, 'job')
      row.append(element('strong', job.request.title || '网页剪藏'))
      row.append(
        element(
          'span',
          (
            {
              capturing: '正在提取',
              uploading: '正在传入墨池',
              failed: '保存失败',
              saved: '已保存',
            } as const
          )[job.status]
        )
      )
      if (job.document) row.append(element('div', job.document, 'hint'))
      if (job.error) row.append(element('div', job.error, 'error'))
      if (job.warnings.length)
        row.append(element('div', `${job.warnings.length} 张图片保留原链接`, 'hint'))
      if (job.status === 'failed') {
        const retry = element('button', '重试')
        retry.onclick = async () => {
          await rpc({ type: 'retry', id: job.id })
          await showJobs()
        }
        row.append(retry)
      }
      if (['saved', 'failed'].includes(job.status)) {
        const remove = element('button', '移除记录')
        remove.onclick = async () => {
          await rpc({ type: 'delete', id: job.id })
          await showJobs()
        }
        row.append(remove)
      }
      box.append(row)
    }
    if (!records.length) box.append(element('p', '保存的文章会显示在这里。', 'hint'))
  } catch (error) {
    message.textContent = String(error)
  }
}
async function init() {
  const saved = await chrome.storage.local.get('format')
  if (typeof saved.format === 'string' && ['markdown', 'html', 'pdf'].includes(saved.format))
    format.value = saved.format
  help()
  ;[tab] = await chrome.tabs.query({ active: true, currentWindow: true })
  $<HTMLInputElement>('title').value = tab?.title || ''
  if (!tab?.id || !tab.url?.startsWith('http')) throw new Error('请在普通网页上使用剪藏')
  try {
    const [state] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => ({
        selection: !!window.getSelection()?.toString().trim(),
        origins: [
          ...new Set(
            [...document.images].flatMap(img => {
              try {
                const u = new URL(img.currentSrc || img.src || img.dataset.src || '', location.href)
                return ['http:', 'https:'].includes(u.protocol) ? [u.origin + '/*'] : []
              } catch {
                return []
              }
            })
          ),
        ],
      }),
    })
    if (state.result) {
      mode.querySelector<HTMLOptionElement>('option[value="selection"]')!.disabled =
        !state.result.selection
      imageOrigins = state.result.origins
    }
  } catch {
    mode.querySelector<HTMLOptionElement>('option[value="selection"]')!.disabled = true
  }
  context = await native<Context>('context')
  $('workspace').textContent = context.name
  $('default').textContent =
    `存入默认知识库 · ${context.defaults.directory.split('/').slice(1).join(' / ')}`
  for (const id of ['inbox', 'default', 'custom']) $<HTMLButtonElement>(id).disabled = false
}
void init().catch(error => {
  message.textContent = String(error)
  $('workspace').textContent = '未连接 · 请打开设置完成绑定'
})
void showJobs()
setInterval(() => void showJobs(), 1500)
