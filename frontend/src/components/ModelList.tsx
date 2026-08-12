import { useEffect, useMemo, useState } from 'react'
import { modelsApi } from '../api/keys'
import type { ManagedModel } from '../api/types'
import { toast } from '../lib/toast'
import { PowerIcon, SearchIcon } from './icons'

export default function ModelList() {
  const [models, setModels] = useState<ManagedModel[]>([])
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [busyId, setBusyId] = useState<string | null>(null)

  const load = () => {
    setLoading(true)
    modelsApi.list().then(setModels).catch((error) => toast(error instanceof Error ? error.message : '模型加载失败', 'err')).finally(() => setLoading(false))
  }
  useEffect(load, [])

  const filtered = useMemo(() => {
    const value = query.trim().toLowerCase()
    return models.filter((model) => !value || model.id.toLowerCase().includes(value) || model.owned_by.toLowerCase().includes(value))
  }, [models, query])

  const toggle = async (model: ManagedModel) => {
    setBusyId(model.id)
    try {
      await modelsApi.setEnabled(model.id, !model.enabled)
      setModels((current) => current.map((item) => item.id === model.id ? { ...item, enabled: !item.enabled } : item))
      toast(`${model.id} 已${model.enabled ? '禁用' : '启用'}`, 'ok')
    } catch (error) {
      toast(error instanceof Error ? error.message : '状态修改失败', 'err')
    } finally {
      setBusyId(null)
    }
  }

  return <>
    <div className="toolbar">
      <div className="topbar-title">模型列表 <span className="count">{models.length} 个</span></div>
      <div className="grow" />
      <div className="search-box" style={{ position: 'relative', width: 280 }}>
        <SearchIcon size={14} style={{ position: 'absolute', left: 11, top: '50%', transform: 'translateY(-50%)', color: 'var(--faint)' }} />
        <input className="input" style={{ paddingLeft: 32 }} placeholder="搜索模型" value={query} onChange={(event) => setQuery(event.target.value)} />
      </div>
    </div>
    <div className="panel row-list">
      {!loading && filtered.length === 0 && <div className="empty"><div className="big">◫</div><p>{models.length ? '没有匹配的模型' : '暂无模型，请先在账号管理中执行连通性测试'}</p></div>}
      {loading && <div className="empty"><p>模型加载中…</p></div>}
      {filtered.map((model) => <div className={`key-row ${model.enabled ? '' : 'disabled'}`} key={model.id} style={{ cursor: 'default' }}>
        <span className={`led ${model.enabled ? 'ok' : ''}`} />
        <div className="key-name"><span className="nm mono">{model.id}</span>{!model.enabled && <span className="disabled-badge">已禁用</span>}</div>
        <div className="key-url">{model.owned_by || 'unknown'}</div>
        <div className="tags" />
        <div className="meta-num">{model.account_count} 个账号支持</div>
        <div className="row-actions"><button className={`btn btn-sm ${model.enabled ? '' : 'btn-enable'}`} disabled={busyId === model.id} onClick={() => void toggle(model)}><PowerIcon size={13} /> {model.enabled ? '禁用' : '启用'}</button></div>
      </div>)}
    </div>
  </>
}
