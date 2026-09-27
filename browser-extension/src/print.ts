import { store } from './db'
const id = new URL(location.href).searchParams.get('id')
if (id) {
  const data = await store<{ html: string; ready: boolean }>('print', 'readonly', s => s.get(id))
  if (data) {
    const parsed = new DOMParser().parseFromString(data.html, 'text/html')
    document.head.append(...parsed.head.querySelectorAll('style'))
    document.body.replaceChildren(...Array.from(parsed.body.childNodes))
    await document.fonts.ready
    await Promise.race([
      Promise.all(
        [...document.images].map(img =>
          img.complete
            ? Promise.resolve()
            : new Promise<void>(resolve => {
                img.onload = () => resolve()
                img.onerror = () => resolve()
              })
        )
      ),
      new Promise(r => setTimeout(r, 10000)),
    ])
    await store('print', 'readwrite', s => s.put({ ...data, ready: true }, id))
  }
}
