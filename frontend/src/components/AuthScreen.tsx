import { useState } from 'react'
import { auth } from '../api/keys'
import { toast } from '../lib/toast'
import { KeyIcon, ShieldIcon } from './icons'

interface Props {
  mode: 'setup' | 'login'
  onDone: () => void
}

export default function AuthScreen({ mode, onDone }: Props) {
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErr('')
    if (mode === 'setup' && password.length < 6) {
      setErr('登录密码至少 6 位')
      return
    }
    if (mode === 'setup' && password !== confirm) {
      setErr('两次输入不一致')
      return
    }
    setBusy(true)
    try {
      if (mode === 'setup') {
        await auth.setup(password)
        toast('已创建账号并登录', 'ok')
      } else {
        await auth.login(password)
      }
      onDone()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
    } finally {
      setBusy(false)
    }
  }

  const isSetup = mode === 'setup'

  return (
    <div className="auth-wrap">
      <form className="auth-card" onSubmit={submit}>
        <div className="auth-mark">
          {isSetup ? <ShieldIcon size={26} /> : <KeyIcon size={26} />}
        </div>
        <h1>{isSetup ? '创建你的账号' : '请先登录'}</h1>
        <p className="tagline">
          {isSetup
            ? '首次使用：设置登录密码，用它加密你所有的 API Key。'
            : '输入登录密码，恢复所有账号与密钥。'}
        </p>

        <div className="field">
          <label htmlFor="pw">登录密码</label>
          <input
            id="pw"
            className="input"
            type="password"
            autoFocus
            autoComplete="current-password"
            placeholder="至少 6 位"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </div>

        {isSetup && (
          <div className="field">
            <label htmlFor="pw2">确认登录密码</label>
            <input
              id="pw2"
              className="input"
              type="password"
              autoComplete="new-password"
              placeholder="再输入一遍"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
            />
          </div>
        )}

        <p className="auth-err">{err}</p>

        <button className="btn btn-primary btn-block" type="submit" disabled={busy || !password}>
          {busy ? '处理中…' : isSetup ? '创建并登录' : '登录'}
        </button>

        {!isSetup && (
          <p className="mono-note">
            <ShieldIcon size={11} style={{ verticalAlign: '-1px' }} /> 登录状态会在服务重启后自动恢复；退出登录后需重新输入密码。
          </p>
        )}
      </form>
    </div>
  )
}
