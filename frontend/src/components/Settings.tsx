import { useEffect, useState } from 'react'
import { auth, clientKeysApi } from '../api/keys'
import type { ClientApiKey, ClientApiKeyCreated } from '../api/types'
import { toast } from '../lib/toast'
import CopyButton from './CopyButton'
import { KeyIcon, PlusIcon, ShieldIcon, TrashIcon } from './icons'

export default function Settings() {
  const [oldPw, setOldPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const [clientKeys, setClientKeys] = useState<ClientApiKey[]>([])
  const [clientKeyName, setClientKeyName] = useState('')
  const [keyBusy, setKeyBusy] = useState(false)
  const [keyErr, setKeyErr] = useState('')
  const [createdKey, setCreatedKey] = useState<ClientApiKeyCreated | null>(null)

  const loadClientKeys = async () => {
    try {
      setClientKeys(await clientKeysApi.list())
    } catch (e) {
      setKeyErr(e instanceof Error ? e.message : '访问密钥加载失败')
    }
  }

  useEffect(() => {
    loadClientKeys()
  }, [])

  const createClientKey = async (e: React.FormEvent) => {
    e.preventDefault()
    setKeyErr('')
    setKeyBusy(true)
    try {
      const created = await clientKeysApi.create(clientKeyName)
      setCreatedKey(created)
      setClientKeyName('')
      await loadClientKeys()
      toast('客户端访问密钥已创建', 'ok')
    } catch (e) {
      setKeyErr(e instanceof Error ? e.message : '创建失败')
    } finally {
      setKeyBusy(false)
    }
  }

  const removeClientKey = async (key: ClientApiKey) => {
    if (!window.confirm(`撤销访问密钥「${key.name}」？使用它的程序将立即无法调用。`)) return
    try {
      await clientKeysApi.remove(key.id)
      setClientKeys((items) => items.filter((item) => item.id !== key.id))
      toast('访问密钥已撤销', 'ok')
    } catch (e) {
      toast(e instanceof Error ? e.message : '撤销失败', 'err')
    }
  }

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

      <div className="panel" style={{ padding: 20, marginBottom: 16 }}>
        <div className="settings-heading">
          <div>
            <div style={{ fontWeight: 600 }}>客户端访问密钥</div>
            <div className="small">其他程序调用 /v1/* 时必须作为 Bearer Token 提交。</div>
          </div>
          <KeyIcon size={18} />
        </div>

        <form onSubmit={createClientKey} className="client-key-form">
          <div className="field">
            <label>名称</label>
            <input
              className="input"
              placeholder="例如：Cherry Studio"
              maxLength={80}
              value={clientKeyName}
              onChange={(e) => setClientKeyName(e.target.value)}
            />
          </div>
          <p className="auth-err">{keyErr}</p>
          <button className="btn btn-primary" type="submit" disabled={keyBusy || !clientKeyName.trim()}>
            <PlusIcon size={13} /> {keyBusy ? '创建中…' : '创建访问密钥'}
          </button>
        </form>

        {createdKey && (
          <div className="created-key">
            <div>
              <strong>请立即保存此密钥</strong>
              <div className="small">出于安全考虑，关闭此提示后将无法再次查看完整内容。</div>
            </div>
            <code>{createdKey.api_key}</code>
            <div className="created-key-actions">
              <CopyButton text={createdKey.api_key} label="复制密钥" />
              <button className="btn btn-sm" type="button" onClick={() => setCreatedKey(null)}>已保存</button>
            </div>
          </div>
        )}

        <div className="client-key-list">
          {clientKeys.length === 0 ? (
            <div className="small">尚未创建访问密钥；此时代理接口会拒绝所有调用。</div>
          ) : clientKeys.map((key) => (
            <div className="client-key-row" key={key.id}>
              <div className="client-key-info">
                <strong>{key.name}</strong>
                <code>{key.prefix}</code>
                <span className="small">
                  创建于 {formatTime(key.created_at)} · {key.last_used_at ? `最后使用 ${formatTime(key.last_used_at)}` : '尚未使用'}
                </span>
              </div>
              <button className="btn btn-danger btn-sm" type="button" onClick={() => removeClientKey(key)}>
                <TrashIcon size={12} /> 撤销
              </button>
            </div>
          ))}
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

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString()
}
