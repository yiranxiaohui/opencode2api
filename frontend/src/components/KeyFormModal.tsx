import { useState } from 'react'
import type { KeyInput, KeySummary, ProxyRecord } from '../api/types'
import { XIcon } from './icons'

interface Props {
  initial?: KeySummary
  proxies?: ProxyRecord[]
  busy?: boolean
  onClose: () => void
  onSave: (input: KeyInput) => void
}

export default function KeyFormModal({ initial, proxies = [], busy, onClose, onSave }: Props) {
  const [name, setName] = useState(initial?.name ?? '')
  const [apiKey, setApiKey] = useState('')
  const [tags, setTags] = useState(initial?.tags.join(', ') ?? '')
  const [notes, setNotes] = useState(initial?.notes ?? '')
  const [proxyId, setProxyId] = useState(initial?.proxy_id ?? '')
  const [accountType, setAccountType] = useState<'normal' | 'go'>(initial?.account_type ?? 'normal')
  const [err, setErr] = useState('')

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    setErr('')
    if (!initial && !apiKey.trim()) return setErr('请填写 API Key（编辑时留空则保留原 Key）')

    onSave({
      name: name.trim(),
      api_key: apiKey.trim() || undefined,
      tags: tags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean),
      notes: notes.trim(),
      account_type: accountType,
      proxy_id: proxyId || null,
    })
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <form onSubmit={submit}>
          <div className="modal-head">
            <h3>{initial ? '编辑账号' : '新增账号'}</h3>
            <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="关闭">
              <XIcon size={15} />
            </button>
          </div>
          <div className="modal-body">
            <div className="field">
              <label>账号名称（可选）</label>
              <input
                className="input"
                placeholder="留空将根据 API Key 自动生成"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
              />
            </div>
            <div className="field">
              <label>API Key {initial ? '（留空保持不变）' : '*'}</label>
              <input
                className="input mono-input"
                type="password"
                autoComplete="off"
                placeholder={initial ? 'sk-…（留空保留原 Key）' : 'sk-…'}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
              />
            </div>
            <div className="field">
              <label>账号类型</label>
              <select className="input" value={accountType} onChange={(e) => setAccountType(e.target.value as 'normal' | 'go')}>
                <option value="normal">普通账号</option>
                <option value="go">Go 订阅账号</option>
              </select>
            </div>
            <div className="field">
              <label>标签</label>
              <input
                className="input"
                placeholder="用逗号分隔，如 LLM, 国内, 备用"
                value={tags}
                onChange={(e) => setTags(e.target.value)}
              />
            </div>
            <div className="field">
              <label>出口代理</label>
              <select className="input" value={proxyId} onChange={(e) => setProxyId(e.target.value)}>
                <option value="">（直连，不使用代理）</option>
                {proxies.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
              <span className="hint">请求 OpenCode 时经所选代理出网，可去「代理池」管理</span>
            </div>
            <div className="field">
              <label>备注</label>
              <input
                className="input"
                placeholder="可选"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
              />
            </div>
            {err && <p className="auth-err" style={{ marginTop: 12 }}>{err}</p>}
          </div>
          <div className="modal-foot">
            <button type="button" className="btn" onClick={onClose}>
              取消
            </button>
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {busy ? '保存中…' : '保存'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
