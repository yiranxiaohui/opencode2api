import { useCallback, useEffect, useRef, useState } from 'react'
import { browserLoginApi } from '../api/keys'
import type { BrowserLoginSession, KeyRecord, ProxyRecord } from '../api/types'
import { XIcon } from './icons'
import RemoteBrowserDesktop from './RemoteBrowserDesktop'

interface Props {
  proxies: ProxyRecord[]
  onClose: () => void
  onImported: (record: KeyRecord) => void
}

export default function BrowserLoginModal({ proxies, onClose, onImported }: Props) {
  const [name, setName] = useState('')
  const [proxyId, setProxyId] = useState('')
  const [accountType, setAccountType] = useState<'normal' | 'go'>('normal')
  const [session, setSession] = useState<BrowserLoginSession | null>(null)
  const [starting, setStarting] = useState(false)
  const [capturing, setCapturing] = useState(false)
  const [closing, setClosing] = useState(false)
  const [connected, setConnected] = useState(false)
  const [error, setError] = useState('')
  const closedRef = useRef(false)
  const capturingRef = useRef(false)
  const automaticAttemptedRef = useRef(false)

  useEffect(() => {
    const id = session?.id
    return () => {
      if (id) void browserLoginApi.stop(id).catch(() => undefined)
    }
  }, [session?.id])

  const close = async () => {
    if (closing) return
    closedRef.current = true
    setClosing(true)
    if (session) {
      await browserLoginApi.stop(session.id).catch(() => undefined)
    }
    onClose()
  }

  const start = async () => {
    setStarting(true)
    setError('')
    try {
      const next = await browserLoginApi.start({
        name: name.trim() || undefined,
        proxy_id: proxyId || null,
        account_type: accountType,
      })
      if (closedRef.current) {
        await browserLoginApi.stop(next.id).catch(() => undefined)
        return
      }
      automaticAttemptedRef.current = false
      setConnected(false)
      setSession(next)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '启动登录窗口失败')
    } finally {
      setStarting(false)
    }
  }

  const capture = useCallback(async (automatic = false) => {
    if (!session || capturingRef.current) return
    capturingRef.current = true
    setCapturing(true)
    setError('')
    try {
      const record = await browserLoginApi.capture(session.id)
      setSession(null)
      onImported(record)
      onClose()
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : '读取 Cookie 失败'
      setError(automatic ? `已检测到登录状态，但自动导入失败：${message}` : message)
    } finally {
      capturingRef.current = false
      setCapturing(false)
    }
  }, [onClose, onImported, session])

  useEffect(() => {
    if (!session || !connected || automaticAttemptedRef.current) return
    let disposed = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const poll = async () => {
      try {
        const status = await browserLoginApi.status(session.id)
        if (disposed) return
        if (status.ready) {
          automaticAttemptedRef.current = true
          await capture(true)
          return
        }
      } catch {
        // The VNC disconnect handler reports terminal session failures.
      }
      if (!disposed) timer = setTimeout(() => void poll(), 1500)
    }
    timer = setTimeout(() => void poll(), 1000)
    return () => {
      disposed = true
      if (timer) clearTimeout(timer)
    }
  }, [capture, connected, session])

  return (
    <div className="modal-overlay remote-browser-overlay">
      <div className={`modal ${session ? 'remote-browser-modal' : ''}`}>
        <div className="modal-head">
          <h3>{session ? '登录 OpenCode' : '网页登录并自动导入'}</h3>
          <button type="button" className="btn btn-ghost btn-sm" disabled={closing} onClick={() => void close()} aria-label="关闭">
            <XIcon size={16} />
          </button>
        </div>

        {!session ? (
          <form onSubmit={(event) => { event.preventDefault(); void start() }}>
            <div className="modal-body">
              <p className="small browser-login-intro">
                系统会在服务器的临时虚拟桌面中打开 Chromium。请在下一个窗口手动登录；进入 workspace 后会自动读取 Cookie 并导入。
              </p>
              <label className="field"><span>账号名称（可选）</span><input className="input" value={name} onChange={(event) => setName(event.target.value)} placeholder="自动使用账号邮箱或 workspace" /></label>
              <label className="field"><span>账号类型</span><select className="input" value={accountType} onChange={(event) => setAccountType(event.target.value as 'normal' | 'go')}><option value="normal">普通账号</option><option value="go">Go 订阅账号</option></select></label>
              <label className="field"><span>绑定代理</span><select className="input" value={proxyId} onChange={(event) => setProxyId(event.target.value)}><option value="">直连</option>{proxies.map((proxy) => <option key={proxy.id} value={proxy.id}>{proxy.name}</option>)}</select><span className="hint">所选代理同时用于浏览器登录、Cookie 验证和账号后续请求；固定出口代理会保持同一 IP。</span></label>
              {error && <p className="browser-login-error" role="alert">{error}</p>}
              <p className="small browser-login-security">浏览器配置只保存在临时目录，不会写入数据卷；会话结束或 15 分钟超时后自动销毁。</p>
            </div>
            <div className="modal-foot">
              <button type="button" className="btn" disabled={starting || closing} onClick={() => void close()}>取消</button>
              <button className="btn btn-primary" disabled={starting || closing}>{starting ? '正在启动 Chromium…' : '打开登录窗口'}</button>
            </div>
          </form>
        ) : (
          <>
            <RemoteBrowserDesktop session={session} onConnectionChange={setConnected} />
            {error && <p className="browser-login-error remote-browser-error" role="alert">{error}</p>}
            <div className="modal-foot remote-browser-foot">
              <span className="small">进入 workspace 后会自动导入；按钮可用于手动重试。密码和 Cookie 不会显示在管理页面中。</span>
              <button type="button" className="btn" disabled={capturing || closing} onClick={() => void close()}>{closing ? '正在关闭…' : '取消'}</button>
              <button type="button" className="btn btn-primary" disabled={!connected || capturing || closing} onClick={() => void capture()}>{capturing ? '正在验证并导入…' : '立即读取并导入'}</button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
