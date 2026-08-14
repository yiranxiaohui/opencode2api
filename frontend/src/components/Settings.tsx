import { useEffect, useState } from 'react'
import { adminTokensApi, auth } from '../api/keys'
import type { AdminToken, AdminTokenCreated, AdminTokenScope } from '../api/types'
import { toast } from '../lib/toast'
import CopyButton from './CopyButton'
import { KeyIcon, ShieldIcon, TrashIcon } from './icons'

const READ_SCOPE: AdminTokenScope = 'admin:read'
const WRITE_SCOPE: AdminTokenScope = 'admin:write'

export default function Settings() {
  const [oldPw, setOldPw] = useState('')
  const [newPw, setNewPw] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  const [tokens, setTokens] = useState<AdminToken[]>([])
  const [tokensLoading, setTokensLoading] = useState(true)
  const [tokenName, setTokenName] = useState('')
  const [tokenPassword, setTokenPassword] = useState('')
  const [tokenScopes, setTokenScopes] = useState<AdminTokenScope[]>([READ_SCOPE, WRITE_SCOPE])
  const [tokenBusy, setTokenBusy] = useState(false)
  const [tokenError, setTokenError] = useState('')
  const [createdToken, setCreatedToken] = useState('')
  const [revoking, setRevoking] = useState<AdminToken | null>(null)
  const [revokePassword, setRevokePassword] = useState('')
  const [revokeBusy, setRevokeBusy] = useState(false)

  useEffect(() => {
    let live = true
    adminTokensApi
      .list()
      .then((items) => { if (live) setTokens(items) })
      .catch((cause) => { if (live) setTokenError(cause instanceof Error ? cause.message : '管理令牌加载失败') })
      .finally(() => { if (live) setTokensLoading(false) })
    return () => { live = false }
  }, [])

  const changePw = async (event: React.FormEvent) => {
    event.preventDefault()
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
    } catch (cause) {
      setErr(cause instanceof Error ? cause.message : '修改失败')
    } finally {
      setBusy(false)
    }
  }

  const createToken = async (event: React.FormEvent) => {
    event.preventDefault()
    setTokenError('')
    if (tokenScopes.length === 0) return setTokenError('请至少选择一项权限')
    setTokenBusy(true)
    try {
      const created = await adminTokensApi.create(tokenName.trim(), tokenPassword, tokenScopes)
      setTokens((current) => [tokenSummary(created), ...current])
      setCreatedToken(created.token)
      setTokenName('')
      setTokenPassword('')
      toast('管理令牌已创建，请立即复制保存', 'ok')
    } catch (cause) {
      setTokenError(cause instanceof Error ? cause.message : '创建失败')
    } finally {
      setTokenBusy(false)
    }
  }

  const toggleScope = (scope: AdminTokenScope) => {
    setTokenScopes((current) =>
      current.includes(scope) ? current.filter((item) => item !== scope) : [...current, scope],
    )
  }

  const revokeToken = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!revoking) return
    setRevokeBusy(true)
    try {
      await adminTokensApi.revoke(revoking.id, revokePassword)
      setTokens((current) => current.filter((token) => token.id !== revoking.id))
      setRevoking(null)
      setRevokePassword('')
      toast('管理令牌已撤销', 'ok')
    } catch (cause) {
      toast(cause instanceof Error ? cause.message : '撤销失败', 'err')
    } finally {
      setRevokeBusy(false)
    }
  }

  return (
    <div className="settings-page">
      <div className="panel settings-status-panel">
        <div className="auth-mark settings-mark">
          <ShieldIcon size={20} />
        </div>
        <div>
          <div className="settings-title">网页登录</div>
          <div className="small">
            当前浏览器使用独立安全会话；退出网页不会影响客户端访问密钥或管理 API Token。
          </div>
        </div>
      </div>

      <div className="panel settings-panel">
        <div className="settings-section-head">
          <div>
            <h2>管理 API Token</h2>
            <p className="small">通过 Authorization: Bearer 调用管理接口。服务器只保存 Token 哈希。</p>
          </div>
          <div className="auth-mark settings-mark"><KeyIcon size={19} /></div>
        </div>

        {createdToken && (
          <div className="admin-token-created" role="status">
            <div>
              <strong>请立即复制这个 Token</strong>
              <p className="small">关闭后无法再次查看，只能重新创建。</p>
            </div>
            <div className="admin-token-secret">
              <code>{createdToken}</code>
              <CopyButton text={createdToken} label="复制" />
              <button className="btn btn-sm" type="button" onClick={() => setCreatedToken('')}>我已保存</button>
            </div>
          </div>
        )}

        <form className="admin-token-form" onSubmit={createToken}>
          <div className="field">
            <label htmlFor="admin-token-name">名称</label>
            <input id="admin-token-name" className="input" maxLength={80} placeholder="例如：部署脚本" value={tokenName} onChange={(event) => setTokenName(event.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="admin-token-password">当前登录密码</label>
            <input id="admin-token-password" className="input" type="password" autoComplete="current-password" value={tokenPassword} onChange={(event) => setTokenPassword(event.target.value)} />
          </div>
          <div className="field admin-token-scope-field">
            <label>权限范围</label>
            <label className="check"><input type="checkbox" checked={tokenScopes.includes(READ_SCOPE)} onChange={() => toggleScope(READ_SCOPE)} /> 读取管理数据</label>
            <label className="check"><input type="checkbox" checked={tokenScopes.includes(WRITE_SCOPE)} onChange={() => toggleScope(WRITE_SCOPE)} /> 修改管理数据</label>
          </div>
          <button className="btn btn-primary" type="submit" disabled={tokenBusy || !tokenName.trim() || !tokenPassword || tokenScopes.length === 0}>
            {tokenBusy ? '创建中…' : '创建管理 Token'}
          </button>
        </form>
        {tokenError && <p className="auth-err" role="alert">{tokenError}</p>}

        <div className="admin-token-list">
          <div className="admin-token-list-head"><strong>已创建的 Token</strong><span className="count">{tokens.length} 条</span></div>
          {tokensLoading ? (
            <div className="admin-token-empty small">正在加载…</div>
          ) : tokens.length === 0 ? (
            <div className="admin-token-empty small">暂无管理 Token</div>
          ) : tokens.map((token) => (
            <div className="admin-token-row" key={token.id}>
              <div><strong>{token.name}</strong><div className="small mono">{token.prefix}</div></div>
              <div className="admin-token-scopes">
                {token.scopes.map((scope) => <span className="tag" key={scope}>{scope === READ_SCOPE ? '读取' : '修改'}</span>)}
              </div>
              <div className="small">
                创建于 {formatTime(token.created_at)}<br />
                {token.last_used_at ? `最后使用 ${formatTime(token.last_used_at)}` : '尚未使用'}
              </div>
              <button className="btn btn-danger btn-sm" type="button" onClick={() => { setRevoking(token); setRevokePassword('') }}>
                <TrashIcon size={12} /> 撤销
              </button>
            </div>
          ))}
        </div>
      </div>

      <div className="panel settings-panel">
        <div className="settings-section-head"><h2>修改登录密码</h2></div>
        <form onSubmit={changePw}>
          <div className="field">
            <label>当前登录密码</label>
            <input className="input" type="password" autoComplete="current-password" value={oldPw} onChange={(event) => setOldPw(event.target.value)} />
          </div>
          <div className="field">
            <label>新登录密码</label>
            <input className="input" type="password" autoComplete="new-password" value={newPw} onChange={(event) => setNewPw(event.target.value)} />
          </div>
          <div className="field">
            <label>确认新登录密码</label>
            <input className="input" type="password" autoComplete="new-password" value={confirm} onChange={(event) => setConfirm(event.target.value)} />
          </div>
          <p className="auth-err">{err}</p>
          <button className="btn btn-primary" type="submit" disabled={busy || !oldPw || !newPw}>
            {busy ? '处理中…' : '修改登录密码（重新加密全部密钥）'}
          </button>
        </form>
      </div>

      {revoking && (
        <div className="modal-overlay" onClick={() => !revokeBusy && setRevoking(null)}>
          <form className="modal" onSubmit={revokeToken} onClick={(event) => event.stopPropagation()}>
            <div className="modal-body" style={{ paddingTop: 24 }}>
              <h3 style={{ margin: '0 0 8px' }}>撤销管理 Token「{revoking.name}」？</h3>
              <p className="small">使用该 Token 的自动化程序会立即失去访问权限。请输入当前登录密码确认。</p>
              <div className="field" style={{ marginBottom: 0 }}>
                <label htmlFor="revoke-token-password">当前登录密码</label>
                <input id="revoke-token-password" className="input" type="password" autoComplete="current-password" autoFocus value={revokePassword} onChange={(event) => setRevokePassword(event.target.value)} />
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" type="button" disabled={revokeBusy} onClick={() => setRevoking(null)}>取消</button>
              <button className="btn btn-danger" type="submit" disabled={revokeBusy || !revokePassword}>{revokeBusy ? '撤销中…' : '确认撤销'}</button>
            </div>
          </form>
        </div>
      )}
    </div>
  )
}

function tokenSummary(created: AdminTokenCreated): AdminToken {
  return {
    id: created.id,
    name: created.name,
    prefix: created.prefix,
    scopes: created.scopes,
    created_at: created.created_at,
    last_used_at: created.last_used_at,
  }
}

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString()
}
