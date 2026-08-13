export type ToastKind = 'info' | 'ok' | 'err'

export interface ToastItem {
  id: number
  kind: ToastKind
  text: string
}

type Listener = (t: ToastItem) => void

let listeners: Listener[] = []
let counter = 0

export function subscribe(fn: Listener) {
  listeners.push(fn)
  return () => {
    listeners = listeners.filter((l) => l !== fn)
  }
}

export function toast(text: string, kind: ToastKind = 'info') {
  const item: ToastItem = { id: ++counter, kind, text }
  listeners.forEach((l) => l(item))
}

function copyWithSelection(text: string) {
  const textarea = document.createElement('textarea')
  const activeElement = document.activeElement
  textarea.value = text
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.opacity = '0'
  textarea.style.pointerEvents = 'none'
  document.body.appendChild(textarea)
  textarea.focus({ preventScroll: true })
  textarea.select()
  textarea.setSelectionRange(0, textarea.value.length)

  try {
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    textarea.remove()
    if (activeElement instanceof HTMLElement) activeElement.focus({ preventScroll: true })
  }
}

function copyWithSelectionAndToast(text: string, label: string) {
  const copied = copyWithSelection(text)
  toast(copied ? label : '复制失败，请手动复制', copied ? 'ok' : 'err')
}

export function copyWithToast(text: string, label = '已复制') {
  if (!navigator.clipboard?.writeText) {
    copyWithSelectionAndToast(text, label)
    return
  }

  navigator.clipboard.writeText(text)
    .then(() => toast(label, 'ok'))
    .catch(() => copyWithSelectionAndToast(text, label))
}
