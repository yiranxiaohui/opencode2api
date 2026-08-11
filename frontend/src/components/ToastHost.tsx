import { useEffect, useState } from 'react'
import { subscribe, type ToastItem } from '../lib/toast'

export default function ToastHost() {
  const [items, setItems] = useState<ToastItem[]>([])

  useEffect(
    () =>
      subscribe((t) => {
        setItems((prev) => [...prev, t])
        setTimeout(() => setItems((prev) => prev.filter((x) => x.id !== t.id)), 2600)
      }),
    [],
  )

  return (
    <div className="toast-host" role="status" aria-live="polite">
      {items.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <span className="dot" />
          <span>{t.text}</span>
        </div>
      ))}
    </div>
  )
}
