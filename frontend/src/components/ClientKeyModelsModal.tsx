import { useMemo, useState } from 'react'
import type { ManagedModel } from '../api/types'
import { SearchIcon, XIcon } from './icons'

interface Props {
  name: string
  models: ManagedModel[]
  initial: string[] | null
  busy?: boolean
  onClose: () => void
  onSave: (allowedModels: string[] | null) => void
}

export default function ClientKeyModelsModal({ name, models, initial, busy, onClose, onSave }: Props) {
  const [restricted, setRestricted] = useState(initial !== null)
  const [selected, setSelected] = useState(() => new Set(initial ?? []))
  const [query, setQuery] = useState('')

  const catalog = useMemo(() => {
    const byId = new Map(models.map((model) => [model.id, model]))
    for (const id of initial ?? []) {
      if (!byId.has(id)) {
        byId.set(id, { id, owned_by: 'unknown', account_count: 0, enabled: false })
      }
    }
    return [...byId.values()].sort((left, right) => left.id.localeCompare(right.id))
  }, [initial, models])

  const filtered = useMemo(() => {
    const value = query.trim().toLowerCase()
    return catalog.filter((model) => (
      !value
      || model.id.toLowerCase().includes(value)
      || model.owned_by.toLowerCase().includes(value)
    ))
  }, [catalog, query])

  const toggle = (id: string) => {
    setSelected((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const selectEveryEnabledModel = () => {
    setSelected(new Set(catalog.filter((model) => model.enabled).map((model) => model.id)))
  }

  const save = (event: React.FormEvent) => {
    event.preventDefault()
    onSave(restricted ? [...selected].sort() : null)
  }

  return (
    <div className="modal-overlay" onClick={() => !busy && onClose()}>
      <form className="modal client-model-modal" onSubmit={save} onClick={(event) => event.stopPropagation()}>
        <div className="modal-head">
          <div>
            <h3>模型访问范围</h3>
            <span className="small">{name}</span>
          </div>
          <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={onClose} aria-label="关闭">
            <XIcon size={15} />
          </button>
        </div>
        <div className="modal-body">
          <div className="client-model-scope">
            <label>
              <input type="radio" name="model-scope" checked={!restricted} onChange={() => setRestricted(false)} />
              <span><strong>全部模型</strong><small>可使用所有未被全局禁用的模型</small></span>
            </label>
            <label>
              <input type="radio" name="model-scope" checked={restricted} onChange={() => setRestricted(true)} />
              <span><strong>指定模型</strong><small>只有勾选的模型可使用此密钥</small></span>
            </label>
          </div>

          {restricted && <>
            <div className="client-model-toolbar">
              <div className="search-box client-model-search">
                <SearchIcon size={14} />
                <input className="input" placeholder="搜索模型" value={query} onChange={(event) => setQuery(event.target.value)} />
              </div>
              <button className="btn btn-sm" type="button" onClick={selectEveryEnabledModel}>选择全部可用</button>
              <button className="btn btn-ghost btn-sm" type="button" onClick={() => setSelected(new Set())}>清空</button>
            </div>
            <div className="client-model-options">
              {filtered.length === 0 ? (
                <div className="client-model-empty small">{catalog.length ? '没有匹配的模型' : '暂无已同步模型，请先测试上游账号'}</div>
              ) : filtered.map((model) => {
                const checked = selected.has(model.id)
                const unavailable = !model.enabled
                return (
                  <label className={`client-model-option ${unavailable ? 'disabled' : ''}`} key={model.id}>
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={unavailable && !checked}
                      onChange={() => toggle(model.id)}
                    />
                    <span className="mono">{model.id}</span>
                    <small>{unavailable ? '全局已禁用或已下线' : `${model.account_count} 个账号支持`}</small>
                  </label>
                )
              })}
            </div>
            <p className="small client-model-selection">已选择 {selected.size} 个模型</p>
          </>}
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" disabled={busy} onClick={onClose}>取消</button>
          <button type="submit" className="btn btn-primary" disabled={busy || (restricted && selected.size === 0)}>
            {busy ? '保存中…' : '保存范围'}
          </button>
        </div>
      </form>
    </div>
  )
}
