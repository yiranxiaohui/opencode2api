import { useState } from 'react'
import type { CookieImportInput, ProxyRecord } from '../api/types'
import { XIcon } from './icons'

export default function CookieImportModal({ proxies, busy, onClose, onSave }: { proxies: ProxyRecord[]; busy: boolean; onClose: () => void; onSave: (input: CookieImportInput) => void }) {
  const [cookie, setCookie] = useState('')
  const [name, setName] = useState('')
  const [proxyId, setProxyId] = useState('')
  const [accountType, setAccountType] = useState<'normal' | 'go'>('normal')
  return <div className="modal-overlay" onClick={onClose}><form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={(e) => { e.preventDefault(); onSave({ cookie, name: name || undefined, proxy_id: proxyId || null, account_type: accountType }) }}>
    <div className="modal-head"><h3>通过 Cookie 导入账号</h3><button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="关闭"><XIcon size={16} /></button></div>
    <div className="modal-body">
      <label className="field"><span>账号名称（可选）</span><input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="自动使用 workspace 名称" /></label>
      <label className="field"><span>Cookie</span><textarea className="input" required rows={7} value={cookie} onChange={(e) => setCookie(e.target.value)} placeholder="支持 Cookie 请求头、JSON 或 Netscape 格式" /></label>
      <label className="field"><span>账号类型</span><select className="input" value={accountType} onChange={(e) => setAccountType(e.target.value as 'normal' | 'go')}><option value="normal">普通账号</option><option value="go">Go 订阅账号</option></select></label>
      <label className="field"><span>绑定代理</span><select className="input" value={proxyId} onChange={(e) => setProxyId(e.target.value)}><option value="">直连</option>{proxies.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}</select></label>
      <p className="small">Cookie 将加密保存在本机，仅用于发现 API Key 和查询套餐额度，不会进入请求日志或备份导出。</p>
    </div>
    <div className="modal-foot"><button type="button" className="btn" onClick={onClose}>取消</button><button className="btn btn-primary" disabled={busy || !cookie.trim()}>{busy ? '验证中…' : '验证并导入'}</button></div>
  </form></div>
}
