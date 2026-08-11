import { useState } from 'react'
import type { ProxyInput, ProxyRecord } from '../api/types'
import { XIcon } from './icons'

interface Props {
  initial?: ProxyRecord
  busy?: boolean
  onClose: () => void
  onSave: (input: ProxyInput) => void
}

export default function ProxyFormModal({ initial, busy, onClose, onSave }: Props) {
  const [name, setName] = useState(initial?.name ?? '')
  const [url, setUrl] = useState(initial?.url ?? '')
  const [err, setErr] = useState('')

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    setErr('')
    if (!name.trim()) return setErr('请填写名称')
    if (!url.trim()) return setErr('请填写代理 URL')
    if (!/^(http|https|socks5|socks5h):\/\//i.test(url.trim())) {
      return setErr('代理 URL 需以 http://、https:// 或 socks5:// 开头')
    }
    onSave({ name: name.trim(), url: url.trim() })
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <form onSubmit={submit}>
          <div className="modal-head">
            <h3>{initial ? '编辑代理' : '新增代理'}</h3>
            <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="关闭">
              <XIcon size={15} />
            </button>
          </div>
          <div className="modal-body">
            <div className="field">
              <label>名称 *</label>
              <input
                className="input"
                placeholder="如 国内直连、Clash 7890、机场节点"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
              />
            </div>
            <div className="field">
              <label>代理 URL *</label>
              <input
                className="input mono-input"
                placeholder="http://127.0.0.1:7890"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
              />
              <span className="hint">
                支持 http://、https:// 与 socks5://；需要认证时写在 URL 里，如
                <span className="mono"> http://user:pass@host:port</span>
              </span>
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
