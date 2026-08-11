import { useMemo, useState } from 'react'
import type { KeySummary } from '../api/types'
import { BoltIcon, EditIcon, SearchIcon, StarIcon, TrashIcon } from './icons'

interface Props {
  keys: KeySummary[]
  selectedId: string | null
  onOpen: (k: KeySummary) => void
  onTest: (k: KeySummary) => void
  onEdit: (k: KeySummary) => void
  onDelete: (k: KeySummary) => void
  onSetDefault: (id: string) => void
}

export default function KeyList({ keys, selectedId, onOpen, onTest, onEdit, onDelete, onSetDefault }: Props) {
  const [query, setQuery] = useState('')
  const [activeTag, setActiveTag] = useState<string | null>(null)

  const allTags = useMemo(() => {
    const counts = new Map<string, number>()
    keys.forEach((k) => k.tags.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1)))
    return [...counts.entries()].sort((a, b) => b[1] - a[1])
  }, [keys])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    return keys.filter((k) => {
      if (activeTag && !k.tags.includes(activeTag)) return false
      if (!q) return true
      return (
        k.name.toLowerCase().includes(q) || k.notes.toLowerCase().includes(q)
      )
    })
  }, [keys, query, activeTag])

  return (
    <>
      <div className="toolbar">
        <div className="search-box" style={{ position: 'relative', flex: 1, maxWidth: 320 }}>
          <SearchIcon
            size={14}
            style={{ position: 'absolute', left: 11, top: '50%', transform: 'translateY(-50%)', color: 'var(--faint)', pointerEvents: 'none' }}
          />
          <input
            className="input"
            style={{ paddingLeft: 32 }}
            placeholder="搜索账号名称 / 备注"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="grow" style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          <button
            className={`tag-chip ${activeTag === null ? 'on' : ''}`}
            onClick={() => setActiveTag(null)}
          >
            全部
          </button>
          {allTags.map(([t, n]) => (
            <button key={t} className={`tag-chip ${activeTag === t ? 'on' : ''}`} onClick={() => setActiveTag(activeTag === t ? null : t)}>
              {t} <span className="n">{n}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="panel row-list">
        {filtered.length === 0 && (
          <div className="empty">
            <div className="big">🗝</div>
            <p>{keys.length === 0 ? '还没有任何账号' : '没有匹配的结果'}</p>
          </div>
        )}
        {filtered.map((k) => (
          <div
            key={k.id}
            className={`key-row ${selectedId === k.id ? 'selected' : ''}`}
            tabIndex={0}
            role="button"
            onClick={() => onOpen(k)}
            onKeyDown={(e) => e.key === 'Enter' && onOpen(k)}
          >
            <span className="led ok" title="已连通" />
            <div className="key-name">
              <span className="nm">{k.name}</span>
              {k.is_default && <span className="default-badge">默认</span>}
              {k.proxy_name && <span className="proxy-badge">🌐 {k.proxy_name}</span>}
            </div>
            <div className="key-url">OpenCode 官方账号</div>
            <div className="tags">
              {k.tags.slice(0, 3).map((t) => (
                <span key={t} className="tag">
                  {t}
                </span>
              ))}
              {k.tags.length > 3 && <span className="meta-num">+{k.tags.length - 3}</span>}
            </div>
            <div className="meta-num">{k.model_count > 0 ? `${k.model_count} 模型` : ''}</div>
            <div className="row-actions" onClick={(e) => e.stopPropagation()}>
              <button className="btn btn-sm" title="连通性测试" onClick={() => onTest(k)}>
                <BoltIcon size={13} />
              </button>
              {!k.is_default && (
                <button className="btn btn-sm" title="设为默认" onClick={() => onSetDefault(k.id)}>
                  <StarIcon size={13} />
                </button>
              )}
              <button className="btn btn-sm" title="编辑" onClick={() => onEdit(k)}>
                <EditIcon size={13} />
              </button>
              <button className="btn btn-sm btn-danger" title="删除" onClick={() => onDelete(k)}>
                <TrashIcon size={13} />
              </button>
            </div>
          </div>
        ))}
      </div>
    </>
  )
}
