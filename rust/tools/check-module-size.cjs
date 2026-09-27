#!/usr/bin/env node
// 限制重构后的原生模块大小，统计时排除生成文件。
const fs = require('node:fs');
const path = require('node:path');

const MODULE_LIMIT = 1500;
const FACADES = {
  'mochi-app/src/app.rs': 800,
  'mochi-app/src/shell.rs': 500,
  'mochi-app/src/view.rs': 200,
  'mochi-app/src/ui/document.rs': 200,
  'mochi-app/src/ui/assistant.rs': 200,
  'mochi-app/src/ui/base_view.rs': 200,
  'mochi-app/src/ui/home.rs': 200,
  'mochi-app/src/ui/ai_markdown.rs': 200,
  'mochi-app/src/ui/rich.rs': 200,
  'mochi-app/src/ui/workflows.rs': 400,
  'mochi-blocks/src/sync.rs': 200,
};

// 此文件不属于本次拆分范围。固定其额度，避免继续增长。
const LEGACY_LIMITS = { 'mochi-app/src/app/file_approvals.rs': 1667 };

function countLines(source) {
  if (!source) return 0;
  return source.split('\n').length - Number(source.endsWith('\n'));
}

function limitFor(file) {
  return FACADES[file] ?? LEGACY_LIMITS[file] ?? MODULE_LIMIT;
}

function overBudget(files) {
  return files.filter(({ file, lines }) => lines > limitFor(file));
}

function collectFiles(crates) {
  const files = new Set();
  function walk(relative) {
    for (const entry of fs.readdirSync(path.join(crates, relative), { withFileTypes: true })) {
      const child = `${relative}/${entry.name}`;
      if (entry.isDirectory()) walk(child);
      else if (entry.isFile() && entry.name.endsWith('.rs')) files.add(child);
    }
  }
  for (const file of Object.keys(FACADES)) {
    if (!fs.statSync(path.join(crates, file)).isFile()) throw new Error(`Missing facade: ${file}`);
    files.add(file);
    walk(file.slice(0, -3));
  }
  walk('mochi-core/src/workflows');
  return [...files].sort().map(file => ({
    file,
    lines: countLines(fs.readFileSync(path.join(crates, file), 'utf8')),
  }));
}

function main() {
  const files = collectFiles(path.resolve(__dirname, '../crates'));
  const violations = overBudget(files);
  for (const { file, lines } of violations) {
    console.error(`${file}: ${lines} lines > ${limitFor(file)}; extract a responsibility or test module.`);
  }
  if (violations.length) process.exitCode = 1;
  else console.log(`Native module-size check passed (${files.length} files, implementation limit ${MODULE_LIMIT}).`);
}

if (require.main === module) main();
module.exports = { countLines, limitFor, overBudget };
