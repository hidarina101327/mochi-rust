import { Readability } from '@mozilla/readability'
import DOMPurify from 'dompurify'
import TurndownService from 'turndown'
import { gfm } from 'turndown-plugin-gfm'
import type { Extracted, Mode } from './types'

export function sanitize(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
      'form',
      'input',
      'button',
      'textarea',
      'select',
      'iframe',
      'object',
      'embed',
      'style',
      'link',
      'meta',
      'base',
    ],
    FORBID_ATTR: ['srcset', 'style', 'id', 'name'],
  })
}
globalThis.__mochiExtract = (mode: Mode): Extracted => {
  let html = '',
    printHtml = '',
    title = document.title,
    author: string | null = null
  if (mode === 'selection') {
    const selection = window.getSelection()
    if (!selection?.rangeCount || selection.isCollapsed) throw new Error('请先选择需要收藏的内容')
    const range = selection.getRangeAt(0)
    const box = document.createElement('div')
    box.append(range.cloneContents())
    html = box.innerHTML
    // 将源元素的样式同步到选区副本中，不改动当前 DOM。
    const source = [...document.querySelectorAll('*')].filter(el => range.intersectsNode(el))
    const clone = [...box.querySelectorAll<HTMLElement>('*')]
    let from = 0
    const properties = [
      'font-family',
      'font-size',
      'font-weight',
      'font-style',
      'line-height',
      'color',
      'background-color',
      'text-align',
      'white-space',
      'border',
      'padding',
      'margin',
      'display',
      'list-style-type',
    ]
    for (const element of clone) {
      const index = source.findIndex(
        (s, i) =>
          i >= from && s.tagName === element.tagName && s.textContent === element.textContent
      )
      if (index < 0) continue
      from = index + 1
      const style = getComputedStyle(source[index])
      for (const key of properties) element.style.setProperty(key, style.getPropertyValue(key))
    }
    for (const img of box.querySelectorAll('img')) {
      const source = img.getAttribute('src') || img.dataset.src
      try {
        if (source) img.src = new URL(source, location.href).href
      } catch {
        img.remove()
      }
      img.removeAttribute('srcset')
    }
    for (const a of box.querySelectorAll('a[href]')) {
      try {
        a.setAttribute('href', new URL(a.getAttribute('href')!, location.href).href)
      } catch {
        a.removeAttribute('href')
      }
    }
    printHtml = DOMPurify.sanitize(box.innerHTML, {
      USE_PROFILES: { html: true },
      FORBID_TAGS: [
        'script',
        'iframe',
        'object',
        'embed',
        'form',
        'input',
        'button',
        'link',
        'meta',
        'base',
      ],
    })
  } else {
    const clone = document.cloneNode(true) as Document
    ;[...clone.images].forEach((img, i) => {
      const original = document.images[i]
      if (original?.currentSrc) img.src = original.currentSrc
      else if (img.dataset.src) img.src = img.dataset.src
    })
    const article = new Readability(clone).parse()
    if (!article?.content || !article.textContent?.trim())
      throw new Error('无法识别文章正文，请改用选区或截图')
    html = article.content
    title = article.title || title
    author = article.byline || null
    printHtml = html
  }
  const box = document.createElement('div')
  box.innerHTML = sanitize(html)
  for (const a of box.querySelectorAll('a[href]')) {
    try {
      const u = new URL(a.getAttribute('href')!, location.href)
      if (['http:', 'https:', 'mailto:'].includes(u.protocol)) a.setAttribute('href', u.href)
      else a.removeAttribute('href')
    } catch {
      a.removeAttribute('href')
    }
  }
  const images: Extracted['images'] = []
  for (const img of box.querySelectorAll('img')) {
    const src = img.getAttribute('src') || img.dataset.src
    if (!src) {
      img.remove()
      continue
    }
    try {
      const url = new URL(src, location.href)
      if (!['http:', 'https:', 'data:'].includes(url.protocol)) {
        img.remove()
        continue
      }
      const token = `mochi-asset-${images.length}-end`
      images.push({ token, url: url.href })
      img.src = token
      img.removeAttribute('srcset')
    } catch {
      img.remove()
    }
  }
  const turndown = new TurndownService({ headingStyle: 'atx', codeBlockStyle: 'fenced' })
  turndown.use(gfm)
  return {
    title,
    url: location.href,
    author,
    html: box.innerHTML,
    markdown: turndown.turndown(box),
    printHtml,
    excerpt: box.textContent?.trim().slice(0, 800) || '',
    images,
  }
}
