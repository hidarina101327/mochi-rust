import { readdir, readFile, mkdir, writeFile } from 'node:fs/promises'
import { zipSync } from 'fflate'
const files = {}
async function collect(root, relative = '') {
  for (const entry of await readdir(`${root}/${relative}`, { withFileTypes: true })) {
    const name = relative + entry.name
    if (entry.isDirectory()) await collect(root, `${name}/`)
    else files[name] = new Uint8Array(await readFile(`${root}/${name}`))
  }
}
await collect('dist')
for (const name of ['README.md'])
  files[name] = new Uint8Array(await readFile(name))
const { version } = JSON.parse(await readFile('package.json', 'utf8'))
await mkdir('artifacts', { recursive: true })
for (const browser of ['chrome', 'edge'])
  await writeFile(
    `artifacts/mochi-web-clipper-${browser}-${version}.zip`,
    zipSync(files, { level: 9 })
  )
console.log('独立市场 ZIP 已写入 artifacts/；不会加入 Rust 安装包。')
