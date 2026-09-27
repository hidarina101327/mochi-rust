import { build } from 'esbuild'
import { mkdir, copyFile, readdir, readFile, writeFile, stat } from 'node:fs/promises'
import path from 'node:path'
await mkdir('dist', { recursive: true })
await build({
  entryPoints: ['src/background.ts', 'src/popup.ts', 'src/options.ts', 'src/print.ts'],
  outdir: 'dist',
  bundle: true,
  format: 'esm',
  target: 'chrome125',
  minify: true,
})
await build({
  entryPoints: ['src/extract.ts'],
  outfile: 'dist/extract.js',
  bundle: true,
  format: 'iife',
  target: 'chrome125',
  minify: true,
})
await copyFile('manifest.json', 'dist/manifest.json')
for (const file of await readdir('public')) await copyFile(`public/${file}`, `dist/${file}`)
await mkdir('dist/icons', { recursive: true })
for (const size of [16, 32, 48, 128]) await copyFile(`icons/${size}.png`, `dist/icons/${size}.png`)
const seen = new Set()
const notices = []
async function license(name, from = process.cwd()) {
  let dir
  let current = from
  while (true) {
    const candidate = path.join(current, 'node_modules', name)
    if (await stat(path.join(candidate, 'package.json')).catch(() => false)) {
      dir = candidate
      break
    }
    const next = path.dirname(current)
    if (next === current) throw new Error(`缺少依赖 ${name}`)
    current = next
  }
  const pkg = JSON.parse(await readFile(path.join(dir, 'package.json'), 'utf8'))
  const key = `${pkg.name}@${pkg.version}`
  if (seen.has(key)) return
  seen.add(key)
  const output = path.join('dist/licenses', key.replaceAll('/', '__'))
  await mkdir(output, { recursive: true })
  const names = (await readdir(dir)).filter(name => /^(licen[cs]e|copying|notice)/i.test(name))
  for (const file of names)
    if ((await stat(path.join(dir, file))).isFile())
      await copyFile(path.join(dir, file), path.join(output, file))
  notices.push({ name: pkg.name, version: pkg.version, license: pkg.license, files: names })
  for (const child of Object.keys(pkg.dependencies || {})) await license(child, dir)
}
for (const name of Object.keys(JSON.parse(await readFile('package.json', 'utf8')).dependencies))
  await license(name)
await writeFile('dist/licenses/index.json', JSON.stringify(notices, null, 2))
console.log('可在 Chrome / Edge 加载 browser-extension/dist')
