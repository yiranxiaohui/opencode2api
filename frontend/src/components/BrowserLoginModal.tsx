import RFB from '@novnc/novnc'
import { useCallback, useEffect, useRef, useState } from 'react'
import { browserLoginApi } from '../api/keys'
import type { BrowserLoginSession, KeyRecord, ProxyRecord } from '../api/types'
import { XIcon } from './icons'

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
  const [clipboardOpen, setClipboardOpen] = useState(false)
  const [clipboardNotice, setClipboardNotice] = useState('')
  const [error, setError] = useState('')
  const closedRef = useRef(false)
  const capturingRef = useRef(false)
  const automaticAttemptedRef = useRef(false)
  const remoteRfbRef = useRef<RFB | null>(null)
  const clipboardInputRef = useRef<HTMLTextAreaElement>(null)

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
      setClipboardOpen(false)
      setClipboardNotice('')
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

  const handleConnected = useCallback(() => setConnected(true), [])
  const handleDisconnected = useCallback((message?: string) => {
    setConnected(false)
    if (message) setError(message)
  }, [])

  const handleRfbChange = useCallback((rfb: RFB | null) => {
    remoteRfbRef.current = rfb
  }, [])

  const closeClipboard = () => {
    if (clipboardInputRef.current) clipboardInputRef.current.value = ''
    setClipboardOpen(false)
  }

  const sendClipboard = () => {
    const rfb = remoteRfbRef.current
    const text = clipboardInputRef.current?.value ?? ''
    if (!rfb || !connected) {
      setError('远程浏览器尚未连接，暂时无法发送剪贴板')
      return
    }
    if (!text) {
      setError('请先把本机内容粘贴到文本框')
      clipboardInputRef.current?.focus()
      return
    }
    try {
      rfb.clipboardPasteFrom(text)
      if (clipboardInputRef.current) clipboardInputRef.current.value = ''
      setClipboardOpen(false)
      setClipboardNotice('已发送到远程剪贴板；请点击远程输入框后按 Ctrl+V')
      setError('')
      rfb.focus()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '发送远程剪贴板失败')
    }
  }

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
            <div className="remote-browser-help">
              <span><i className={connected ? 'connected' : ''} />{connected ? '远程浏览器已连接' : '正在连接远程浏览器…'}</span>
              <span>会话将在 {new Date(session.expires_at * 1000).toLocaleTimeString()} 过期</span>
            </div>
            <RemoteBrowser sessionId={session.id} onConnected={handleConnected} onDisconnected={handleDisconnected} onRfbChange={handleRfbChange} />
            {clipboardOpen && (
              <div className="remote-browser-clipboard">
                <textarea
                  ref={clipboardInputRef}
                  className="input"
                  rows={2}
                  autoFocus
                  aria-label="要发送到远程浏览器的文本"
                  placeholder="在这里粘贴本机文本（内容只会发送到当前远程浏览器）"
                  onKeyDown={(event) => {
                    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
                      event.preventDefault()
                      sendClipboard()
                    }
                  }}
                />
                <div className="remote-browser-clipboard-actions">
                  <span className="small">粘贴后发送，再回到远程输入框按 Ctrl+V；Ctrl/⌘+Enter 可直接发送。</span>
                  <button type="button" className="btn btn-sm" onClick={closeClipboard}>取消</button>
                  <button type="button" className="btn btn-primary btn-sm" onClick={sendClipboard}>发送到远程剪贴板</button>
                </div>
              </div>
            )}
            {error && <p className="browser-login-error remote-browser-error" role="alert">{error}</p>}
            <div className="modal-foot remote-browser-foot">
              <span className={`small ${clipboardNotice ? 'remote-browser-clipboard-notice' : ''}`}>{clipboardNotice || '进入 workspace 后会自动导入；按钮可用于手动重试。密码和 Cookie 不会显示在管理页面中。'}</span>
              <button type="button" className="btn" disabled={!connected || capturing || closing} onClick={() => { setClipboardNotice(''); setClipboardOpen((open) => !open) }}>{clipboardOpen ? '收起粘贴框' : '粘贴本机文本'}</button>
              <button type="button" className="btn" disabled={capturing || closing} onClick={() => void close()}>{closing ? '正在关闭…' : '取消'}</button>
              <button type="button" className="btn btn-primary" disabled={!connected || capturing || closing} onClick={() => void capture()}>{capturing ? '正在验证并导入…' : '立即读取并导入'}</button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}

function RemoteBrowser({ sessionId, onConnected, onDisconnected, onRfbChange }: { sessionId: string; onConnected: () => void; onDisconnected: (message?: string) => void; onRfbChange: (rfb: RFB | null) => void }) {
  const targetRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const target = targetRef.current
    if (!target) return
    let disposed = false
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const url = `${protocol}//${window.location.host}/api/browser-login/${encodeURIComponent(sessionId)}/vnc`
    const rfb = new RFB(target, url)
    onRfbChange(rfb)
    rfb.scaleViewport = true
    rfb.resizeSession = false
    rfb.clipViewport = false
    rfb.focusOnClick = true
    rfb.qualityLevel = 7
    rfb.compressionLevel = 5
    rfb.addEventListener('connect', () => {
      if (!disposed) onConnected()
    })
    rfb.addEventListener('disconnect', (event) => {
      if (!disposed) onDisconnected(event.detail.clean ? undefined : '远程浏览器连接已断开，请关闭窗口后重试')
    })
    rfb.addEventListener('securityfailure', (event) => {
      if (!disposed) onDisconnected(event.detail.reason || '远程浏览器安全协商失败')
    })
    rfb.focus()
    return () => {
      disposed = true
      onRfbChange(null)
      rfb.disconnect()
      target.replaceChildren()
    }
  }, [sessionId, onConnected, onDisconnected, onRfbChange])

  return <div className="remote-browser-screen" ref={targetRef} />
}
