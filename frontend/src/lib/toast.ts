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

export function copyWithToast(text: string, label = '已复制') {
  navigator.clipboard
    .writeText(text)
    .then(() => toast(label, 'ok'))
    .catch(() => toast('复制失败', 'err'))
}
