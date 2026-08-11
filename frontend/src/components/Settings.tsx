import { useState } from 'react'
import { auth } from '../api/keys'
import { toast } from '../lib/toast'
import { ShieldIcon } from './icons'

export default function Settings() {
  const [oldPw, setOldPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const changePw = async (e: React.FormEvent) => {
    e.preventDefault()
    setErr('')
    if (newPw.length < 6) return setErr('新密码至少 6 位')
    if (newPw !== confirm) return setErr('两次输入不一致')
    setBusy(true)
    try {
      await auth.changePassword(oldPw, newPw)
      setOldPw('')
      setNewPw('')
      setConfirm('')
      toast('登录密码已更新，全部密钥已重新加密', 'ok')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '修改失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div style={{ maxWidth: 720 }}>
      <div className="panel" style={{ padding: 20, marginBottom: 16, display: 'flex', gap: 14, alignItems: 'center' }}>
        <div className="auth-mark" style={{ margin: 0, width: 42, height: 42 }}>
          <ShieldIcon size={20} />
        </div>
        <div>
          <div style={{ fontWeight: 600, marginBottom: 2 }}>登录状态</div>
          <div className="small">
            API Key 使用 AES-256-GCM 加密保存；服务重启后会自动恢复，无需重新登录。
          </div>
        </div>
      </div>

      <div className="panel" style={{ padding: 20 }}>
        <form onSubmit={changePw}>
          <div className="field">
            <label>当前登录密码</label>
            <input className="input" type="password" autoComplete="current-password" value={oldPw} onChange={(e) => setOldPw(e.target.value)} />
          </div>
          <div className="field">
            <label>新登录密码</label>
            <input className="input" type="password" autoComplete="new-password" value={newPw} onChange={(e) => setNewPw(e.target.value)} />
          </div>
          <div className="field">
            <label>确认新登录密码</label>
            <input className="input" type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
          </div>
          <p className="auth-err">{err}</p>
          <button className="btn btn-primary" type="submit" disabled={busy || !oldPw || !newPw}>
            {busy ? '处理中…' : '修改登录密码（重新加密全部密钥）'}
          </button>
        </form>
      </div>

    </div>
  )
}
