import RFB from '@novnc/novnc'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { BrowserLoginSession } from '../api/types'

interface Props {
  session: BrowserLoginSession
  onConnectionChange?: (connected: boolean) => void
}

export default function RemoteBrowserDesktop({ session, onConnectionChange }: Props) {
  const [connected, setConnected] = useState(false)
  const [clipboardOpen, setClipboardOpen] = useState(false)
  const [clipboardNotice, setClipboardNotice] = useState('')
  const [error, setError] = useState('')
  const remoteRfbRef = useRef<RFB | null>(null)
  const clipboardInputRef = useRef<HTMLTextAreaElement>(null)

  const handleConnected = useCallback(() => {
    setConnected(true)
    setError('')
    onConnectionChange?.(true)
  }, [onConnectionChange])

  const handleDisconnected = useCallback((message?: string) => {
    setConnected(false)
    onConnectionChange?.(false)
    if (message) setError(message)
  }, [onConnectionChange])

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
    <>
      <div className="remote-browser-help">
        <span><i className={connected ? 'connected' : ''} />{connected ? '远程浏览器已连接' : '正在连接远程浏览器…'}</span>
        <div className="remote-browser-help-actions">
          <span>会话将在 {new Date(session.expires_at * 1000).toLocaleTimeString()} 过期</span>
          <button type="button" className="btn btn-sm" disabled={!connected} onClick={() => { setClipboardNotice(''); setClipboardOpen((open) => !open) }}>{clipboardOpen ? '收起粘贴框' : '粘贴本机文本'}</button>
        </div>
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
      {clipboardNotice && <p className="remote-browser-clipboard-notice small">{clipboardNotice}</p>}
      {error && <p className="browser-login-error remote-browser-error" role="alert">{error}</p>}
    </>
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
