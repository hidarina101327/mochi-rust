import type { Job } from './types'
const opened: Promise<IDBDatabase> = new Promise((resolve, reject) => {
  const r = indexedDB.open('mochi-web-clipper', 1)
  r.onupgradeneeded = () => {
    r.result.createObjectStore('jobs', { keyPath: 'id' })
    r.result.createObjectStore('print')
  }
  r.onsuccess = () => resolve(r.result)
  r.onerror = () => reject(r.error)
})
export async function store<T>(
  table: string,
  mode: IDBTransactionMode,
  operation: (s: IDBObjectStore) => IDBRequest<T>
): Promise<T> {
  const db = await opened
  return new Promise((resolve, reject) => {
    const tx = db.transaction(table, mode)
    const r = operation(tx.objectStore(table))
    tx.oncomplete = () => resolve(r.result)
    tx.onerror = () => reject(tx.error)
    tx.onabort = () => reject(tx.error)
  })
}
export const putJob = (job: Job) => store('jobs', 'readwrite', s => s.put(job))
export const getJob = (id: string) => store<Job | undefined>('jobs', 'readonly', s => s.get(id))
export const jobs = () => store<Job[]>('jobs', 'readonly', s => s.getAll())
export const deleteJob = (id: string) => store('jobs', 'readwrite', s => s.delete(id))
