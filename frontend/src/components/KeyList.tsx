import { useMemo, useState } from 'react'
import { useQueries } from '@tanstack/react-query'
import { keysApi } from '../api/keys'
import type { KeySummary } from '../api/types'
import { BoltIcon, EditIcon, PowerIcon, SearchIcon, StarIcon, TrashIcon } from './icons'

interface Props {
  keys: KeySummary[]
  selectedId: string | null
  onOpen: (k: KeySummary) => void
  onTest: (k: KeySummary) => void
  onEdit: (k: KeySummary) => void
  onDelete: (k: KeySummary) => void
  onSetDefault: (id: string) => void
  onSetEnabled: (id: string, enabled: boolean) => void
}

export default function KeyList({ keys, selectedId, onOpen, onTest, onEdit, onDelete, onSetDefault, onSetEnabled }: Props) {
  const [query, setQuery] = useState('')
  const [activeTag, setActiveTag] = useState<string | null>(null)
  const usageQueries = useQueries({ queries: keys.map((key) => ({
    queryKey: ['account-usage', key.id],
    queryFn: () => keysApi.usage(key.id),
    enabled: key.has_cookie && key.is_enabled,
    staleTime: 60_000,
    refetchInterval: 5 * 60_000,
    retry: 1,
  })) })
  const usageById = new Map(keys.map((key, index) => [key.id, usageQueries[index]]))

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
            className={`key-row ${selectedId === k.id ? 'selected' : ''} ${!k.is_enabled ? 'disabled' : ''}`}
            tabIndex={0}
            role="button"
            onClick={() => onOpen(k)}
            onKeyDown={(e) => e.key === 'Enter' && onOpen(k)}
          >
            <span className={`led ${k.is_enabled ? 'ok' : ''}`} title={k.is_enabled ? '已启用' : '已禁用'} />
            <div className="key-name">
              <span className="nm">{k.name}</span>
              {k.is_default && <span className="default-badge">默认</span>}
              {!k.is_enabled && <span className="disabled-badge">已禁用</span>}
              {k.proxy_name && <span className="proxy-badge">🌐 {k.proxy_name}</span>}
            </div>
            <div className="key-usage">
              {!k.has_cookie && <span className="key-url">API Key 账号</span>}
              {k.has_cookie && usageById.get(k.id)?.isPending && <span className="key-url">正在查询额度…</span>}
              {k.has_cookie && usageById.get(k.id)?.isError && <span className="usage-error">额度查询失败</span>}
              {usageById.get(k.id)?.data && <>
                <span className="plan-name">{usageById.get(k.id)!.data!.plan_name}</span>
                <div className="usage-pills">
                  {usageById.get(k.id)!.data!.rolling && <span>滚 {usageById.get(k.id)!.data!.rolling!.remaining_percent.toFixed(0)}%</span>}
                  {usageById.get(k.id)!.data!.weekly && <span>周 {usageById.get(k.id)!.data!.weekly!.remaining_percent.toFixed(0)}%</span>}
                  {usageById.get(k.id)!.data!.monthly && <span>月 {usageById.get(k.id)!.data!.monthly!.remaining_percent.toFixed(0)}%</span>}
                </div>
              </>}
            </div>
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
              {k.is_enabled && !k.is_default && (
                <button className="btn btn-sm" title="设为默认" onClick={() => onSetDefault(k.id)}>
                  <StarIcon size={13} />
                </button>
              )}
              <button
                className={`btn btn-sm ${k.is_enabled ? '' : 'btn-enable'}`}
                title={k.is_enabled ? '禁用账号' : '启用账号'}
                aria-label={k.is_enabled ? `禁用账号 ${k.name}` : `启用账号 ${k.name}`}
                onClick={() => onSetEnabled(k.id, !k.is_enabled)}
              >
                <PowerIcon size={13} />
              </button>
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
