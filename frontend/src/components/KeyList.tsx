import { useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { keysApi } from '../api/keys'
import type { KeySummary } from '../api/types'
import { keysQueryKey } from '../hooks/useKeys'
import { toast } from '../lib/toast'
import { BoltIcon, EditIcon, PowerIcon, RefreshIcon, SearchIcon, TrashIcon } from './icons'

interface Props {
  keys: KeySummary[]
  selectedId: string | null
  onOpen: (k: KeySummary) => void
  onTest: (k: KeySummary) => void
  onEdit: (k: KeySummary) => void
  onDelete: (k: KeySummary) => void
  onSetEnabled: (id: string, enabled: boolean) => void
}

export default function KeyList({ keys, selectedId, onOpen, onTest, onEdit, onDelete, onSetEnabled }: Props) {
  const [query, setQuery] = useState('')
  const [activeTag, setActiveTag] = useState<string | null>(null)
  const [queryingIds, setQueryingIds] = useState<Set<string>>(() => new Set())
  const queryClient = useQueryClient()

  const loadUsage = async (key: KeySummary) => {
    setQueryingIds((current) => new Set(current).add(key.id))
    try {
      const usage = await keysApi.usage(key.id)
      queryClient.setQueryData<KeySummary[]>(keysQueryKey, (current) =>
        current?.map((item) => item.id === key.id ? { ...item, usage_cache: usage } : item),
      )
      toast(`${key.name} 额度已更新`, 'ok')
    } catch (error) {
      toast(error instanceof Error ? error.message : '额度查询失败', 'err')
    } finally {
      setQueryingIds((current) => {
        const next = new Set(current)
        next.delete(key.id)
        return next
      })
    }
  }

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
        {filtered.map((k) => {
          const usage = k.usage_cache
          const isQuerying = queryingIds.has(k.id)
          return (
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
              {!k.is_enabled && <span className="disabled-badge">已禁用</span>}
              {k.proxy_name && <span className="proxy-badge">🌐 {k.proxy_name}</span>}
            </div>
            <div className="key-usage">
              {!k.has_cookie && <span className="key-url">API Key 账号</span>}
              {k.has_cookie && usage && <>
                <span className="plan-name" title={`缓存于 ${new Date(usage.fetched_at * 1000).toLocaleString()}`}>{usage.plan_name}</span>
                <div className="usage-pills">
                  {usage.rolling && <span>滚 {usage.rolling.remaining_percent.toFixed(0)}%</span>}
                  {usage.weekly && <span>周 {usage.weekly.remaining_percent.toFixed(0)}%</span>}
                  {usage.monthly && <span>月 {usage.monthly.remaining_percent.toFixed(0)}%</span>}
                </div>
              </>}
              {k.has_cookie && (
                <button
                  className="usage-query"
                  disabled={isQuerying}
                  title={usage ? '刷新额度缓存' : '查询额度'}
                  onClick={(event) => { event.stopPropagation(); void loadUsage(k) }}
                >
                  {isQuerying && <RefreshIcon size={12} className="spin" />}
                  {isQuerying ? '查询中…' : usage ? '刷新' : '查询额度'}
                </button>
              )}
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
          )
        })}
      </div>
    </>
  )
}
