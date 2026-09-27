import type { Context } from './types'
export async function rpc<T = unknown>(message: Record<string, unknown>): Promise<T> {
  const reply = await chrome.runtime.sendMessage(message)
  if (!reply?.ok) throw new Error(reply?.error || '扩展后台没有响应')
  return reply.result
}
export const native = <T>(op: string, fields: Record<string, unknown> = {}) =>
  rpc<T>({ type: 'native', op, fields })
export function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text?: string,
  className?: string
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag)
  if (text) e.textContent = text
  if (className) e.className = className
  return e
}
export function directoryTree(
  container: HTMLElement,
  context: Context,
  onPick: (id: string, folder: string, label: string) => void
) {
  container.replaceChildren()
  for (const library of context.libraries) {
    branch(container, library.id, '', library.name, library.name)
  }
  if (!context.libraries.length)
    container.append(element('p', '暂时没有知识库。首次“存入默认知识库”会创建浏览器收藏。', 'hint'))
  function branch(parent: HTMLElement, id: string, folder: string, name: string, label: string) {
    const row = element('div', undefined, 'tree-row')
    const toggle = element('button', '›', 'tree-toggle')
    toggle.type = 'button'
    toggle.setAttribute('aria-label', `展开 ${name}`)
    const select = element('button', name, 'tree-select')
    select.type = 'button'
    select.onclick = () => {
      container.querySelectorAll('.selected').forEach(e => e.classList.remove('selected'))
      select.classList.add('selected')
      onPick(id, folder, label)
    }
    row.append(toggle, select)
    parent.append(row)
    const children = element('div', undefined, 'tree-children')
    children.hidden = true
    parent.append(children)
    let loaded = false
    toggle.onclick = async () => {
      children.hidden = !children.hidden
      toggle.textContent = children.hidden ? '›' : '⌄'
      if (loaded || children.hidden) return
      toggle.disabled = true
      try {
        const names = await native<string[]>('folders', {
          workspace: context.workspace,
          libraryId: id,
          folder,
        })
        children.replaceChildren()
        for (const child of names)
          branch(children, id, folder ? `${folder}/${child}` : child, child, `${label} / ${child}`)
        if (!names.length) children.append(element('span', '无子文件夹', 'hint'))
        loaded = true
      } catch (error) {
        children.textContent = String(error)
      } finally {
        toggle.disabled = false
      }
    }
  }
}
