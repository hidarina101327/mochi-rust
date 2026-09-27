import { PDFDocument } from 'pdf-lib'
import { getJob, putJob, jobs, deleteJob, store } from './db'
import {
  nativeRequest,
  sha256,
  encode,
  decode,
  MAX_IMAGE,
  MAX_TOTAL,
  readingHtml,
  escapeHtml,
} from './protocol'
import type { Job, SaveRequest, Extracted, ClipFile, Context } from './types'

const running = new Set<string>()
async function status(job: Job) {
  await putJob(job)
  await chrome.action.setBadgeText({
    text: job.status === 'saved' ? '✓' : job.status === 'failed' ? '!' : '…',
  })
  await chrome.action.setBadgeBackgroundColor({
    color: job.status === 'failed' ? '#b84040' : '#176858',
  })
}
async function boundedImage(url: string): Promise<Blob> {
  const response = await fetch(url, { credentials: 'include', signal: AbortSignal.timeout(20000) })
  if (!response.ok) throw new Error(`图片下载失败 (${response.status})`)
  const mime = response.headers.get('content-type')?.split(';')[0] || ''
  if (!['image/png', 'image/jpeg', 'image/webp', 'image/gif', 'image/avif'].includes(mime))
    throw new Error('不支持的图片格式')
  const reader = response.body?.getReader()
  if (!reader) throw new Error('图片内容为空')
  const parts: Uint8Array[] = []
  let length = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      length += value.length
      if (length > MAX_IMAGE) throw new Error('单张图片超过 20 MiB')
      parts.push(value)
    }
  } catch (error) {
    await reader.cancel()
    throw error
  }
  return new Blob(parts as BlobPart[], { type: mime })
}
async function extract(tabId: number, mode: SaveRequest['mode']): Promise<Extracted> {
  await chrome.scripting.executeScript({ target: { tabId }, files: ['extract.js'] })
  const [result] = await chrome.scripting.executeScript({
    target: { tabId },
    func: m => {
      if (!globalThis.__mochiExtract) throw new Error('正文提取器未加载')
      return globalThis.__mochiExtract(m)
    },
    args: [mode],
  })
  if (!result?.result) throw new Error('无法提取此页面')
  return result.result
}
async function cropRegion(
  tabId: number,
  crop: boolean
): Promise<{
  x: number
  y: number
  width: number
  height: number
  viewportWidth: number
  viewportHeight: number
}> {
  const [r] = await chrome.scripting.executeScript({
    target: { tabId },
    func: async (crop: boolean) => {
      if (!crop)
        return {
          x: 0,
          y: 0,
          width: innerWidth,
          height: innerHeight,
          viewportWidth: innerWidth,
          viewportHeight: innerHeight,
        }
      return new Promise<{
        x: number
        y: number
        width: number
        height: number
        viewportWidth: number
        viewportHeight: number
      }>((resolve, reject) => {
        const overlay = document.createElement('div')
        Object.assign(overlay.style, {
          position: 'fixed',
          inset: '0',
          zIndex: '2147483647',
          cursor: 'crosshair',
          background: 'rgba(0,0,0,.14)',
          touchAction: 'none',
        })
        const frame = document.createElement('div')
        Object.assign(frame.style, {
          position: 'absolute',
          border: '2px solid #30ba94',
          background: 'rgba(255,255,255,.15)',
          pointerEvents: 'none',
        })
        overlay.append(frame)
        const label = document.createElement('div')
        label.textContent = '拖动框选 · Esc 取消'
        Object.assign(label.style, {
          position: 'absolute',
          top: '18px',
          left: '50%',
          transform: 'translateX(-50%)',
          padding: '10px 18px',
          background: '#163a31',
          color: 'white',
          font: '14px system-ui',
          borderRadius: '8px',
        })
        overlay.append(label)
        let start: { x: number; y: number } | null = null
        let timer: ReturnType<typeof setTimeout>
        const cleanup = () => {
          overlay.remove()
          document.removeEventListener('keydown', key, true)
          clearTimeout(timer)
        }
        const key = (e: KeyboardEvent) => {
          if (e.key === 'Escape') {
            e.preventDefault()
            e.stopPropagation()
            cleanup()
            reject(new Error('已取消框选'))
          }
        }
        document.addEventListener('keydown', key, true)
        timer = setTimeout(() => {
          cleanup()
          reject(new Error('框选超时'))
        }, 120000)
        overlay.onpointerdown = e => {
          e.preventDefault()
          start = { x: e.clientX, y: e.clientY }
          overlay.setPointerCapture(e.pointerId)
        }
        overlay.onpointermove = e => {
          if (!start) return
          Object.assign(frame.style, {
            left: `${Math.min(start.x, e.clientX)}px`,
            top: `${Math.min(start.y, e.clientY)}px`,
            width: `${Math.abs(start.x - e.clientX)}px`,
            height: `${Math.abs(start.y - e.clientY)}px`,
          })
        }
        overlay.onpointerup = async e => {
          if (!start) return
          const x = Math.max(0, Math.min(start.x, e.clientX)),
            y = Math.max(0, Math.min(start.y, e.clientY)),
            width = Math.min(innerWidth, Math.max(start.x, e.clientX)) - x,
            height = Math.min(innerHeight, Math.max(start.y, e.clientY)) - y
          cleanup()
          if (width < 4 || height < 4) {
            reject(new Error('框选范围太小'))
            return
          }
          await new Promise(requestAnimationFrame)
          await new Promise(requestAnimationFrame)
          resolve({ x, y, width, height, viewportWidth: innerWidth, viewportHeight: innerHeight })
        }
        document.documentElement.append(overlay)
      })
    },
    args: [crop],
  })
  if (!r?.result) throw new Error('截图失败')
  return r.result
}
async function screenshot(tab: chrome.tabs.Tab, request: SaveRequest): Promise<Blob> {
  const rect = await cropRegion(tab.id!, request.crop)
  const active = await chrome.tabs.query({ active: true, windowId: tab.windowId })
  if (active[0]?.id !== tab.id) throw new Error('截图时页面已切换，请重试')
  const url = await chrome.tabs.captureVisibleTab(tab.windowId, { format: 'png' })
  const image = await createImageBitmap(await (await fetch(url)).blob())
  const sx = image.width / rect.viewportWidth,
    sy = image.height / rect.viewportHeight
  const canvas = new OffscreenCanvas(Math.round(rect.width * sx), Math.round(rect.height * sy))
  canvas
    .getContext('2d')!
    .drawImage(
      image,
      rect.x * sx,
      rect.y * sy,
      rect.width * sx,
      rect.height * sy,
      0,
      0,
      canvas.width,
      canvas.height
    )
  image.close()
  return canvas.convertToBlob({ type: 'image/png' })
}
async function printPdf(tabId: number, html?: string): Promise<Blob> {
  if (!(await chrome.permissions.contains({ permissions: ['debugger'] })))
    throw new Error('请先允许 PDF 所需的调试权限')
  let temporary: number | undefined
  let key: string | undefined
  let attached = false
  try {
    if (html) {
      key = crypto.randomUUID()
      await store('print', 'readwrite', s => s.put({ html, ready: false }, key!))
      const tab = await chrome.tabs.create({
        url: chrome.runtime.getURL(`print.html?id=${key}`),
        active: false,
      })
      temporary = tab.id
      tabId = tab.id!
      let ready = false
      for (let i = 0; i < 150; i++) {
        const state = await store<{ ready: boolean }>('print', 'readonly', s => s.get(key!))
        if (state?.ready) {
          ready = true
          break
        }
        await new Promise(r => setTimeout(r, 100))
      }
      if (!ready) throw new Error('PDF 排版超时')
    }
    await chrome.debugger.attach({ tabId }, '1.3')
    attached = true
    const result = (await chrome.debugger.sendCommand({ tabId }, 'Page.printToPDF', {
      printBackground: true,
      preferCSSPageSize: true,
      transferMode: 'ReturnAsStream',
    })) as { stream?: string; data?: string }
    if (result.data) return new Blob([decode(result.data) as BlobPart], { type: 'application/pdf' })
    if (!result.stream) throw new Error('浏览器未返回 PDF')
    const chunks: Uint8Array[] = []
    let size = 0
    try {
      while (true) {
        const part = (await chrome.debugger.sendCommand({ tabId }, 'IO.read', {
          handle: result.stream,
          size: 196608,
        })) as { data: string; base64Encoded?: boolean; eof: boolean }
        const bytes = part.base64Encoded ? decode(part.data) : new TextEncoder().encode(part.data)
        size += bytes.length
        if (size > MAX_TOTAL) throw new Error('PDF 超过 100 MiB')
        chunks.push(bytes)
        if (part.eof) break
      }
    } finally {
      await chrome.debugger
        .sendCommand({ tabId }, 'IO.close', { handle: result.stream })
        .catch(() => {})
    }
    return new Blob(chunks as BlobPart[], { type: 'application/pdf' })
  } finally {
    if (attached) await chrome.debugger.detach({ tabId }).catch(() => {})
    if (temporary) await chrome.tabs.remove(temporary).catch(() => {})
    if (key) await store('print', 'readwrite', s => s.delete(key!))
  }
}
async function capture(job: Job): Promise<void> {
  const request = job.request
  const tab = await chrome.tabs.get(request.tabId)
  if (!tab.url || !/^https?:\/\//.test(tab.url)) throw new Error('此页面不支持剪藏，请打开普通网页')
  const capturedAt = Date.now()
  let title = request.title?.trim() || tab.title || '网页收藏',
    author: string | null = null,
    excerpt = '',
    html = '',
    markdown = ''
  const files: ClipFile[] = []
  let document: Blob
  if (request.mode === 'screenshot') {
    const png = await screenshot(tab, request)
    if (png.size > MAX_IMAGE) throw new Error('截图超过 20 MiB')
    if (request.format === 'pdf') {
      const pdf = await PDFDocument.create()
      const image = await pdf.embedPng(await png.arrayBuffer())
      const scale = Math.min(1, 14400 / Math.max(image.width, image.height))
      const page = pdf.addPage([image.width * scale, image.height * scale])
      page.drawImage(image, {
        x: 0,
        y: 0,
        width: image.width * scale,
        height: image.height * scale,
      })
      document = new Blob([(await pdf.save()) as BlobPart], { type: 'application/pdf' })
    } else {
      files.push({ name: 'assets/screenshot.png', blob: png })
      html = '<img src="assets/screenshot.png" alt="网页截图">'
      markdown = '![网页截图](assets/screenshot.png)'
      document = new Blob()
    }
  } else {
    const article = await extract(request.tabId, request.mode)
    if (article.images.length > 999) throw new Error('正文图片超过 999 张，请改用选区保存')
    title = request.title?.trim() || article.title
    author = article.author
    excerpt = article.excerpt
    html = article.html
    markdown = article.markdown
    if (request.format === 'pdf')
      document = await printPdf(
        request.tabId,
        request.mode === 'selection'
          ? readingHtml(title, tab.url, article.printHtml, capturedAt)
          : undefined
      )
    else {
      for (const [i, image] of article.images.entries()) {
        let replacement = image.url
        try {
          if (request.downloadImages === false) throw new Error('未启用离线图片')
          if (!image.url.startsWith('data:')) {
            const origin = new URL(image.url).origin + '/*'
            if (!(await chrome.permissions.contains({ origins: [origin] })))
              throw new Error('尚未授权图片站点')
          }
          const blob = await boundedImage(image.url)
          const ext = (
            {
              'image/png': 'png',
              'image/jpeg': 'jpg',
              'image/gif': 'gif',
              'image/webp': 'webp',
              'image/avif': 'avif',
            } as Record<string, string>
          )[blob.type]
          replacement = `assets/image-${i}.${ext}`
          files.push({ name: replacement, blob })
        } catch (e) {
          job.warnings.push(`图片 ${i + 1} 保留原链接：${String(e)}`)
        }
        html = html.replaceAll(image.token, escapeHtml(replacement))
        markdown = markdown.replaceAll(
          image.token,
          replacement.replace(/\(/g, '%28').replace(/\)/g, '%29')
        )
        if (files.reduce((n, f) => n + f.blob.size, 0) > MAX_TOTAL)
          throw new Error('单次剪藏附件超过 100 MiB')
      }
      document = new Blob()
    }
  }
  if (request.format === 'html')
    document = new Blob([readingHtml(title, tab.url, html, capturedAt)], {
      type: 'text/html;charset=utf-8',
    })
  if (request.format === 'markdown')
    document = new Blob(
      [
        `# ${title.replace(/[\r\n]/g, ' ')}\n\n来源：<${tab.url}>\n剪藏时间：${new Date(capturedAt).toISOString()}${author ? `\n作者：${author}` : ''}\n\n${markdown}\n`,
      ],
      { type: 'text/markdown;charset=utf-8' }
    )
  const ext = request.format === 'markdown' ? 'md' : request.format
  files.unshift({ name: `document.${ext}`, blob: document! })
  if (files.reduce((n, f) => n + f.blob.size, 0) > MAX_TOTAL)
    throw new Error('单次剪藏不能超过 100 MiB')
  job.files = files
  job.clip = {
    clipId: job.id,
    workspace: request.workspace,
    title,
    url: tab.url,
    author,
    capturedAt,
    mode: request.mode,
    format: request.format,
    destination: request.destination,
    libraryId: request.libraryId,
    folder: request.folder,
    excerpt,
    files: await Promise.all(
      files.map(async f => ({ name: f.name, size: f.blob.size, sha256: await sha256(f.blob) }))
    ),
  }
  await putJob(job)
}
async function run(id: string) {
  if (running.has(id)) return
  running.add(id)
  const job = await getJob(id)
  if (!job) {
    running.delete(id)
    return
  }
  try {
    if (!job.clip || !job.files) {
      job.status = 'capturing'
      await status(job)
      await capture(job)
    }
    job.status = 'uploading'
    delete job.error
    await status(job)
    const common = { workspace: job.request.workspace, clipId: job.id }
    const existing = await nativeRequest<{ status: string; document?: string }>('result', common)
    if (existing.status === 'saved') {
      job.document = existing.document
    } else {
      await nativeRequest('begin', { workspace: job.request.workspace, clip: job.clip })
      for (const file of job.files!) {
        for (let offset = 0; offset < file.blob.size; offset += 196608) {
          const bytes = new Uint8Array(await file.blob.slice(offset, offset + 196608).arrayBuffer())
          await nativeRequest('chunk', { ...common, name: file.name, offset, data: encode(bytes) })
        }
        if (file.blob.size === 0)
          await nativeRequest('chunk', { ...common, name: file.name, offset: 0, data: '' })
      }
      const result = await nativeRequest<{ status: string; document: string }>('commit', common)
      if (result.status !== 'saved') throw new Error('墨池未确认保存成功')
      job.document = result.document
    }
    job.status = 'saved'
    delete job.files
    delete job.clip
    await status(job)
  } catch (e) {
    job.status = 'failed'
    job.error = e instanceof Error ? e.message : String(e)
    await status(job)
  } finally {
    running.delete(id)
  }
}
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (
    sender.id !== chrome.runtime.id ||
    (sender.tab && !sender.url?.startsWith(chrome.runtime.getURL('')))
  )
    return
  ;(async () => {
    switch (message.type) {
      case 'native': {
        if (!['ping', 'context', 'folders', 'defaults'].includes(message.op))
          throw new Error('未知操作')
        return nativeRequest(message.op, message.fields || {})
      }
      case 'save': {
        const context = await nativeRequest<Context>('context')
        if (context.workspace !== message.request.workspace)
          throw new Error('工作区已切换，请刷新后重试')
        const job: Job = {
          id: crypto.randomUUID(),
          request: message.request,
          created: Date.now(),
          status: 'capturing',
          warnings: [],
        }
        await putJob(job)
        void run(job.id)
        return { id: job.id }
      }
      case 'jobs':
        return (await jobs())
          .sort((a, b) => b.created - a.created)
          .map(({ files, clip, ...job }) => job)
      case 'retry':
        void run(message.id)
        return {}
      case 'delete':
        if (running.has(message.id)) throw new Error('任务仍在保存，请稍后删除')
        await deleteJob(message.id)
        return {}
      default:
        throw new Error('未知扩展操作')
    }
  })().then(
    result => sendResponse({ ok: true, result }),
    error => sendResponse({ ok: false, error: String(error.message || error) })
  )
  return true
})
async function recover() {
  for (const job of await jobs()) {
    if (job.status === 'uploading' && job.clip && job.files) void run(job.id)
    else if (job.status === 'capturing' && !running.has(job.id)) {
      job.status = 'failed'
      job.error = '浏览器中断了内容提取，请回到原页面后重试'
      await status(job)
    }
  }
}
chrome.runtime.onStartup.addListener(() => void recover())
chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm.name === 'recover') void recover()
})
chrome.runtime.onInstalled.addListener(() => {
  void chrome.alarms.create('recover', { periodInMinutes: 1 })
})
