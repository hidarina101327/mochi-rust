// Real Chromium/Edge extension APIs. MOCHI_CLIPPER_REAL_NATIVE=1 registers an
// isolated native host; the default forwards unchanged frames through a fixture.
import { chromium } from '../../node_modules/@playwright/test/index.mjs'
import { createServer } from 'node:http'
import { mkdtemp, cp, readFile, writeFile, mkdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { spawn, execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { generateKeyPairSync, createHash, randomUUID } from 'node:crypto'
import assert from 'node:assert/strict'

const root = path.resolve(import.meta.dirname, '../..')
const realNative = process.env.MOCHI_CLIPPER_REAL_NATIVE === '1'
const execute = promisify(execFile)
const registryKeys = []
const temp = await mkdtemp(path.join(tmpdir(), 'mochi-browser-smoke-'))
const extension = path.join(temp, 'extension'),
  workspace = path.join(temp, 'workspace'),
  bin = path.join(temp, 'bin')
await cp(path.join(root, 'browser-extension/dist'), extension, { recursive: true })
await mkdir(workspace, { recursive: true })
await mkdir(bin, { recursive: true })
const nativeBin =
  process.env.MOCHI_CLIPPER_NATIVE_BIN || path.join(root, 'rust/target-web-clipper/release')
for (const name of ['mochi-app.exe', 'mochi-clipper-host.exe'])
  await cp(path.join(nativeBin, name), path.join(bin, name))
const manifest = JSON.parse(await readFile(path.join(extension, 'manifest.json'), 'utf8'))
manifest.permissions.push('debugger')
manifest.optional_permissions = []
// The fixture invokes saves from an extension tab instead of clicking its toolbar
// action, so it cannot receive activeTab. Grant capture permission in the fixture only.
manifest.host_permissions = ['<all_urls>']
const settings = path.join(temp, 'settings.json')
if (realNative) {
  const key = generateKeyPairSync('rsa', { modulusLength: 2048 }).publicKey.export({
    type: 'spki',
    format: 'der',
  })
  manifest.key = key.toString('base64')
  const id = [...createHash('sha256').update(key).digest('hex').slice(0, 32)]
    .map(c => String.fromCharCode(97 + parseInt(c, 16)))
    .join('')
  await writeFile(
    settings,
    JSON.stringify({
      'webClipper.enabled': 'true',
      'webClipper.extensionIds': id,
      'workspace.lastPath': workspace,
    })
  )
  const name = `com.mochi.clip_smoke_${randomUUID().replaceAll('-', '')}`
  const background = path.join(extension, 'background.js')
  await writeFile(
    background,
    (await readFile(background, 'utf8')).replaceAll('com.mochi.web_clipper', name)
  )
  const hostManifest = path.join(temp, 'native-host.json')
  await writeFile(
    hostManifest,
    JSON.stringify({
      name,
      description: 'Isolated Mochi clipper test',
      path: path.join(bin, 'mochi-clipper-host.exe'),
      type: 'stdio',
      allowed_origins: [`chrome-extension://${id}/`],
    })
  )
  for (const browser of ['Google\\Chrome', 'Microsoft\\Edge']) {
    const registry = `HKCU\\Software\\${browser}\\NativeMessagingHosts\\${name}`
    await execute(
      path.join(process.env.SystemRoot, 'System32/reg.exe'),
      ['add', registry, '/ve', '/t', 'REG_SZ', '/d', hostManifest, '/f'],
      { windowsHide: true }
    )
    registryKeys.push(registry)
  }
}
await writeFile(path.join(extension, 'manifest.json'), JSON.stringify(manifest))
let host, context
const pending = new Map()
let stdout = Buffer.alloc(0),
  stderr = ''
function native(request) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(request.id)
      reject(new Error('Native fixture timed out: ' + stderr))
    }, 45000)
    pending.set(request.id, {
      resolve: v => {
        clearTimeout(timer)
        resolve(v)
      },
      reject,
    })
    const bytes = Buffer.from(JSON.stringify(request))
    const length = Buffer.alloc(4)
    length.writeUInt32LE(bytes.length)
    host.stdin.write(Buffer.concat([length, bytes]))
  })
}
const image = await readFile(path.join(root, 'browser-extension/icons/128.png'))
const article = `<!doctype html><html><head><title>剪藏浏览器验收</title><style>body{font:18px/1.7 sans-serif;max-width:780px;margin:40px auto}p{color:#234}img{width:128px}</style></head><body><article><h1>剪藏浏览器验收</h1><p id="selection">这是选区内容，保留中文与 <strong>加粗格式</strong>。</p>${Array.from({ length: 20 }, (_, i) => `<p>段落 ${i}：验证网页全文保存、本地图片和原排版 PDF。${'这是一段真实浏览器中的长正文。'.repeat(8)}</p>`).join('')}<img src="/image.png"><table><tr><th>项目</th><th>结果</th></tr><tr><td>正文</td><td>完整</td></tr></table></article></body></html>`
const server = createServer(async (req, res) => {
  try {
    if (req.url === '/rpc') {
      let raw = ''
      for await (const part of req) raw += part
      const result = await native(JSON.parse(raw))
      res.writeHead(200, { 'content-type': 'application/json', 'access-control-allow-origin': '*' })
      res.end(JSON.stringify(result))
    } else if (req.url === '/image.png') {
      res.writeHead(200, { 'content-type': 'image/png' })
      res.end(image)
    } else {
      res.writeHead(200, { 'content-type': 'text/html;charset=utf-8' })
      res.end(article)
    }
  } catch (error) {
    res.writeHead(500)
    res.end(String(error))
  }
})
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const origin = `http://127.0.0.1:${server.address().port}`
try {
  const executable =
    process.env.MOCHI_CLIPPER_BROWSER ||
    path.join(process.env.LOCALAPPDATA, 'ms-playwright/chromium-1234/chrome-win64/chrome.exe')
  context = await chromium.launchPersistentContext(path.join(temp, 'profile'), {
    executablePath: executable,
    headless: process.env.MOCHI_CLIPPER_HEADFUL !== '1',
    viewport: { width: 1100, height: 800 },
    env: { ...process.env, MOCHI_SETTINGS_FILE: settings, MOCHI_VERIFY_OFFSCREEN: '1' },
    args: [
      `--disable-extensions-except=${extension}`,
      `--load-extension=${extension}`,
      '--window-position=-32000,-32000',
      '--disable-backgrounding-occluded-windows',
    ],
  })
  const worker = context.serviceWorkers()[0] || (await context.waitForEvent('serviceworker'))
  const id = new URL(worker.url()).host
  if (!realNative) {
    await writeFile(
      settings,
      JSON.stringify({
        'webClipper.enabled': 'true',
        'webClipper.extensionIds': id,
        'workspace.lastPath': workspace,
      })
    )
    host = spawn(path.join(bin, 'mochi-clipper-host.exe'), [`chrome-extension://${id}/`], {
      windowsHide: true,
      env: { ...process.env, MOCHI_SETTINGS_FILE: settings, MOCHI_VERIFY_OFFSCREEN: '1' },
    })
    host.stderr.on('data', data => {
      stderr += data
    })
    host.stdout.on('data', data => {
      stdout = Buffer.concat([stdout, data])
      while (stdout.length >= 4) {
        const len = stdout.readUInt32LE()
        if (stdout.length < 4 + len) break
        const reply = JSON.parse(stdout.subarray(4, 4 + len))
        stdout = stdout.subarray(4 + len)
        pending.get(reply.id)?.resolve(reply)
        pending.delete(reply.id)
      }
    })
    await worker.evaluate(url => {
      chrome.runtime.connectNative = () => {
        const listeners = []
        const disconnect = []
        let closed = false
        return {
          name: 'fixture',
          onMessage: { addListener: f => listeners.push(f) },
          onDisconnect: { addListener: f => disconnect.push(f) },
          postMessage: request => {
            fetch(url, { method: 'POST', body: JSON.stringify(request) })
              .then(r => r.json())
              .then(reply => {
                if (!closed) listeners.forEach(f => f(reply))
              })
              .catch(() => disconnect.forEach(f => f()))
          },
          disconnect: () => {
            closed = true
          },
        }
      }
    }, origin + '/rpc')
  }
  const page = await context.newPage()
  await page.goto(origin + '/article')
  const options = await context.newPage()
  await options.goto(`chrome-extension://${id}/options.html`)
  const message = data =>
    options.evaluate(async data => {
      const r = await chrome.runtime.sendMessage(data)
      if (!r.ok) throw new Error(r.error)
      return r.result
    }, data)
  const info = await message({ type: 'native', op: 'context' })
  assert.ok(info.workspace)
  assert.equal(info.name, 'workspace')
  const tabs = await options.evaluate(() => chrome.tabs.query({}))
  const tabId = tabs.find(t => t.url?.includes('/article')).id
  const cases = [
    ['article', 'markdown', 'inbox'],
    ['article', 'html', 'default'],
    ['article', 'pdf', 'default'],
    ['selection', 'markdown', 'inbox'],
    ['selection', 'html', 'custom'],
    ['selection', 'pdf', 'inbox'],
    ['screenshot', 'markdown', 'inbox'],
    ['screenshot', 'html', 'inbox'],
    ['screenshot', 'pdf', 'inbox'],
    ['screenshot', 'markdown', 'inbox', true],
  ]
  for (const [mode, format, destination, crop = false] of cases) {
    await page.bringToFront()
    if (mode === 'selection')
      await page.evaluate(() => {
        const range = document.createRange()
        range.selectNodeContents(document.querySelector('#selection'))
        window.getSelection().removeAllRanges()
        window.getSelection().addRange(range)
      })
    const request = {
      tabId,
      workspace: info.workspace,
      mode,
      format,
      destination,
      folder: '',
      crop,
      downloadImages: true,
    }
    if (destination === 'custom') {
      const current = await message({ type: 'native', op: 'context' })
      const library = current.libraries.find(l => l.name === '浏览器收藏')
      await mkdir(path.join(library.path, '自定义文件夹'), { recursive: true })
      request.libraryId = library.id
      request.folder = '自定义文件夹'
      const folders = await message({
        type: 'native',
        op: 'folders',
        fields: { workspace: info.workspace, libraryId: library.id, folder: '' },
      })
      assert.ok(folders.includes('自定义文件夹'))
    }
    const saved = await message({ type: 'save', request })
    if (crop) {
      await page.getByText('拖动框选 · Esc 取消').waitFor()
      await page.mouse.move(100, 150)
      await page.mouse.down()
      await page.mouse.move(480, 430, { steps: 8 })
      await page.mouse.up()
    }
    let job
    for (let i = 0; i < 90; i++) {
      job = (await message({ type: 'jobs' })).find(j => j.id === saved.id)
      if (['failed', 'saved'].includes(job?.status)) break
      await new Promise(r => setTimeout(r, 500))
    }
    assert.equal(job?.status, 'saved', `${mode}/${format}: ${job?.error || 'timeout'}`)
    const bytes = await readFile(path.join(workspace, job.document))
    assert.ok(bytes.length > 50)
    if (format === 'pdf') assert.equal(bytes.subarray(0, 5).toString(), '%PDF-')
    if (format === 'markdown' && mode === 'article') {
      assert.ok(bytes.toString().includes('段落 19'))
      assert.ok(bytes.toString().includes('assets/image-0.png'))
    }
    if (mode === 'selection' && format === 'markdown') {
      assert.ok(bytes.toString().includes('这是选区内容'))
      assert.ok(!bytes.toString().includes('段落 19'))
    }
    const again = await message({ type: 'native', op: 'context' })
    assert.equal(again.workspace, info.workspace)
    console.log(`PASS ${mode} / ${format} / ${destination}: ${bytes.length} bytes`)
  }
  await options.bringToFront()
  // Reopening settings refreshes native context without launching a real protocol handler.
  await options.reload()
  await options.waitForFunction(() => document.querySelector('#connection-status')?.textContent === '已连接墨池')
  assert.equal(await options.locator('#extension-id').count(), 0)
  assert.equal(await options.locator('#connect').getAttribute('href'), `mochi-clipper://connect?extension=${id}`)
  await options.waitForFunction(() =>
    document.querySelector('#storage')?.hidden === false
  )
  await mkdir(path.join(root, 'browser-extension/artifacts'), { recursive: true })
  await options.screenshot({
    path: path.join(root, 'browser-extension/artifacts/options-smoke.png'),
    fullPage: true,
  })
  const inbox = JSON.parse(await readFile(path.join(workspace, '收件箱/items.json'), 'utf8'))
  assert.equal(inbox.items.length, cases.filter(c => c[2] === 'inbox').length)
  console.log('PASS native auto-launch, receipt, inbox, offline image, browser PDF and screenshot')
} finally {
  if (host?.pid) {
    await new Promise(resolve => {
      const kill = spawn('taskkill', ['/PID', String(host.pid), '/T', '/F'], {
        windowsHide: true,
        stdio: 'ignore',
      })
      kill.on('exit', resolve)
    })
  }
  if (realNative) {
    const literal = path.join(bin, 'mochi-app.exe').replaceAll("'", "''")
    await execute(
      'powershell.exe',
      [
        '-NoProfile',
        '-Command',
        `Get-CimInstance Win32_Process -Filter "name = 'mochi-app.exe'" | Where-Object { $_.ExecutablePath -eq '${literal}' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }`,
      ],
      { windowsHide: true }
    ).catch(() => {})
    for (const registry of registryKeys)
      await execute(
        path.join(process.env.SystemRoot, 'System32/reg.exe'),
        ['delete', registry, '/f'],
        { windowsHide: true }
      ).catch(() => {})
  }
  await context?.close()
  server.close()
  for (const value of pending.values()) value.reject(new Error('Test closed'))
  if (process.env.MOCHI_CLIPPER_KEEP_SMOKE) console.log(`Fixtures: ${temp}`)
  else {
    const checked = path.resolve(temp)
    if (
      path.dirname(checked) !== path.resolve(tmpdir()) ||
      !path.basename(checked).startsWith('mochi-browser-smoke-')
    )
      throw new Error('Unsafe smoke cleanup path')
    await rm(checked, { recursive: true, force: true, maxRetries: 5, retryDelay: 1000 })
  }
}
