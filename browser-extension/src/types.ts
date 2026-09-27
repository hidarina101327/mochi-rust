export type Mode = 'article' | 'selection' | 'screenshot'
export type Format = 'markdown' | 'html' | 'pdf'
export type Destination = 'inbox' | 'default' | 'custom'
export interface Library {
  id: string
  name: string
  path: string
}
export interface Context {
  workspace: string
  name: string
  libraries: Library[]
  defaults: { directory: string }
}
export interface SaveRequest {
  tabId: number
  workspace: string
  mode: Mode
  format: Format
  destination: Destination
  libraryId?: string
  folder: string
  crop: boolean
  title?: string
  downloadImages?: boolean
}
export interface Extracted {
  title: string
  url: string
  author: string | null
  html: string
  markdown: string
  printHtml: string
  excerpt: string
  images: { token: string; url: string }[]
}
export interface ClipInput {
  clipId: string
  workspace: string
  title: string
  url: string
  author: string | null
  capturedAt: number
  mode: Mode
  format: Format
  destination: Destination
  libraryId?: string
  folder: string
  excerpt: string
  files: { name: string; size: number; sha256: string }[]
}
export interface ClipFile {
  name: string
  blob: Blob
}
export interface Job {
  id: string
  request: SaveRequest
  created: number
  status: 'capturing' | 'uploading' | 'failed' | 'saved'
  error?: string
  warnings: string[]
  clip?: ClipInput
  files?: ClipFile[]
  document?: string
}
declare global {
  var __mochiExtract: ((mode: Mode) => Extracted) | undefined
}
