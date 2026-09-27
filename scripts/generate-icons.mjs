// SVG 的渐变和滤镜需经 Chromium 渲染，再封装为 PNG/ICO/ICNS。

import { chromium } from '@playwright/test'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const svgPath = path.join(root, 'public', 'logo.svg')
const outDir = path.join(root, 'build')

const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
// OSType -> pixel size. PNG-compatible icns entry types (Mac OS X 10.7+).
const ICNS_TYPES = [
  ['ic07', 128],
  ['ic08', 256],
  ['ic09', 512],
  ['ic10', 1024],
  ['ic11', 32],
  ['ic12', 64],
  ['ic13', 256],
  ['ic14', 512],
]
const RENDER_SIZES = [...new Set([1024, ...ICO_SIZES, ...ICNS_TYPES.map(([, s]) => s)])].sort(
  (a, b) => a - b
)

// --- Chromium discovery -----------------------------------------------------
// Prefer Playwright's default resolution; if the bundled build number doesn't
// match what's on disk, fall back to the newest chromium-* in the browser cache.

function browserCacheRoots() {
  const env = process.env.PLAYWRIGHT_BROWSERS_PATH
  if (env && env !== '0') return [env]
  const home = os.homedir()
  if (process.platform === 'win32') {
    return [path.join(process.env.LOCALAPPDATA || path.join(home, 'AppData', 'Local'), 'ms-playwright')]
  }
  if (process.platform === 'darwin') return [path.join(home, 'Library', 'Caches', 'ms-playwright')]
  return [path.join(home, '.cache', 'ms-playwright')]
}

async function findChromiumExecutable() {
  for (const dir of browserCacheRoots()) {
    let entries
    try {
      entries = await fs.readdir(dir)
    } catch {
      continue
    }
    const builds = entries
      .filter((name) => name.startsWith('chromium-'))
      .sort((a, b) => (parseInt(b.slice(9), 10) || 0) - (parseInt(a.slice(9), 10) || 0))
    for (const build of builds) {
      const candidates = [
        path.join(dir, build, 'chrome-win64', 'chrome.exe'),
        path.join(dir, build, 'chrome-win', 'chrome.exe'),
        path.join(dir, build, 'chrome-mac', 'Chromium.app', 'Contents', 'MacOS', 'Chromium'),
        path.join(dir, build, 'chrome-linux', 'chrome'),
      ]
      for (const candidate of candidates) {
        try {
          await fs.access(candidate)
          return candidate
        } catch {
          /* keep looking */
        }
      }
    }
  }
  return null
}

async function launchBrowser() {
  try {
    return await chromium.launch()
  } catch {
    const executablePath = await findChromiumExecutable()
    if (!executablePath) {
      throw new Error('No Chromium found for Playwright. Run: npx playwright install chromium')
    }
    return await chromium.launch({ executablePath })
  }
}

// --- Rendering --------------------------------------------------------------

async function renderAllSizes(svg) {
  const browser = await launchBrowser()
  try {
    const page = await browser.newPage({ deviceScaleFactor: 1 })
    const pngBySize = new Map()
    for (const size of RENDER_SIZES) {
      const html =
        '<!doctype html><meta charset="utf-8">' +
        `<style>*{margin:0;padding:0}html,body{background:transparent}` +
        `#wrap{width:${size}px;height:${size}px}#wrap svg{width:100%;height:100%;display:block}</style>` +
        `<div id="wrap">${svg}</div>`
      await page.setViewportSize({ width: size, height: size })
      await page.setContent(html, { waitUntil: 'networkidle' })
      const el = await page.$('#wrap')
      pngBySize.set(size, await el.screenshot({ omitBackground: true, type: 'png' }))
    }
    return pngBySize
  } finally {
    await browser.close()
  }
}

// --- Encoders ---------------------------------------------------------------

function buildIco(pngBySize) {
  const images = ICO_SIZES.map((size) => ({ size, buf: pngBySize.get(size) }))
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0) // reserved
  header.writeUInt16LE(1, 2) // type: icon
  header.writeUInt16LE(images.length, 4)

  const dir = Buffer.alloc(16 * images.length)
  let offset = header.length + dir.length
  images.forEach((img, i) => {
    const o = i * 16
    const dim = img.size >= 256 ? 0 : img.size // 0 means 256 in the ICO spec
    dir.writeUInt8(dim, o) // width
    dir.writeUInt8(dim, o + 1) // height
    dir.writeUInt8(0, o + 2) // palette size
    dir.writeUInt8(0, o + 3) // reserved
    dir.writeUInt16LE(1, o + 4) // color planes
    dir.writeUInt16LE(32, o + 6) // bits per pixel
    dir.writeUInt32LE(img.buf.length, o + 8) // image size
    dir.writeUInt32LE(offset, o + 12) // image offset
    offset += img.buf.length
  })

  return Buffer.concat([header, dir, ...images.map((i) => i.buf)])
}

function buildIcns(pngBySize) {
  const parts = []
  for (const [type, size] of ICNS_TYPES) {
    const data = pngBySize.get(size)
    const head = Buffer.alloc(8)
    head.write(type, 0, 4, 'ascii')
    head.writeUInt32BE(data.length + 8, 4) // entry length includes the 8-byte header
    parts.push(head, data)
  }
  const body = Buffer.concat(parts)
  const fileHeader = Buffer.alloc(8)
  fileHeader.write('icns', 0, 4, 'ascii')
  fileHeader.writeUInt32BE(body.length + 8, 4)
  return Buffer.concat([fileHeader, body])
}

// --- Main -------------------------------------------------------------------

async function main() {
  const svg = await fs.readFile(svgPath, 'utf-8')
  await fs.mkdir(outDir, { recursive: true })

  console.log(`Rendering ${RENDER_SIZES.length} sizes from ${path.relative(root, svgPath)} ...`)
  const pngBySize = await renderAllSizes(svg)

  await fs.writeFile(path.join(outDir, 'icon.png'), pngBySize.get(1024))
  await fs.writeFile(path.join(outDir, 'icon.ico'), buildIco(pngBySize))
  await fs.writeFile(path.join(outDir, 'icon.icns'), buildIcns(pngBySize))

  console.log('Wrote build/icon.png (1024x1024), build/icon.ico, build/icon.icns')
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
