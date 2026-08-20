import RFB from '@novnc/novnc'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { BrowserLoginSession } from '../api/types'
import { copyWithToast } from '../lib/toast'
import { CopyIcon, PasteIcon } from './icons'

const CONTROL_LEFT_KEYSYM = 0xffe3
const V_KEYSYM = 0x0076
const REMOTE_CLIPBOARD_DELAY_MS = 500

interface Props {
  session: BrowserLoginSession
  onConnectionChange?: (connected: boolean) => void
}

export default function RemoteBrowserDesktop({ session, onConnectionChange }: Props) {
  const [connected, setConnected] = useState(false)
  const [clipboardOpen, setClipboardOpen] = useState(false)
  const [clipboardNotice, setClipboardNotice] = useState('')
  const [remoteClipboard, setRemoteClipboard] = useState('')
  const [error, setError] = useState('')
  const remoteRfbRef = useRef<RFB | null>(null)
  const clipboardInputRef = useRef<HTMLTextAreaElement>(null)
  const pasteTimerRef = useRef<number | null>(null)
  const lastSentClipboardRef = useRef('')

  useEffect(() => () => {
    if (pasteTimerRef.current !== null) window.clearTimeout(pasteTimerRef.current)
  }, [])

  const handleConnected = useCallback(() => {
    setConnected(true)
    setError('')
    onConnectionChange?.(true)
  }, [onConnectionChange])

  const handleDisconnected = useCallback((message?: string) => {
    if (pasteTimerRef.current !== null) {
      window.clearTimeout(pasteTimerRef.current)
      pasteTimerRef.current = null
    }
    lastSentClipboardRef.current = ''
    setClipboardNotice('')
    setConnected(false)
    onConnectionChange?.(false)
    if (message) setError(message)
  }, [onConnectionChange])

  const handleRfbChange = useCallback((rfb: RFB | null) => {
    remoteRfbRef.current = rfb
    if (!rfb) {
      if (pasteTimerRef.current !== null) {
        window.clearTimeout(pasteTimerRef.current)
        pasteTimerRef.current = null
      }
      lastSentClipboardRef.current = ''
    }
  }, [])

  const closeClipboard = () => {
    if (clipboardInputRef.current) clipboardInputRef.current.value = ''
    setClipboardOpen(false)
  }

  const sendClipboard = (providedText?: string) => {
    const rfb = remoteRfbRef.current
    const text = providedText ?? clipboardInputRef.current?.value ?? ''
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
      lastSentClipboardRef.current = text
      rfb.clipboardPasteFrom(text)
      if (clipboardInputRef.current) clipboardInputRef.current.value = ''
      setClipboardOpen(false)
      setClipboardNotice('正在粘贴到远程当前输入框…')
      setError('')
      rfb.focus()
      if (pasteTimerRef.current !== null) window.clearTimeout(pasteTimerRef.current)
      pasteTimerRef.current = window.setTimeout(() => {
        pasteTimerRef.current = null
        if (remoteRfbRef.current !== rfb) return
        let controlPressed = false
        try {
          lastSentClipboardRef.current = ''
          rfb.sendKey(CONTROL_LEFT_KEYSYM, 'ControlLeft', true)
          controlPressed = true
          rfb.sendKey(V_KEYSYM, 'KeyV')
          setClipboardNotice('已粘贴到远程当前输入框')
        } catch (cause) {
          setClipboardNotice('')
          setError(cause instanceof Error ? cause.message : '远程粘贴失败')
        } finally {
          if (controlPressed) {
            try {
              rfb.sendKey(CONTROL_LEFT_KEYSYM, 'ControlLeft', false)
            } catch {
              // The VNC connection is already unavailable.
            }
          }
        }
      }, REMOTE_CLIPBOARD_DELAY_MS)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '发送远程剪贴板失败')
    }
  }

  const openClipboard = async () => {
    setClipboardNotice('')
    if (window.isSecureContext && navigator.clipboard?.readText) {
      try {
        const text = await navigator.clipboard.readText()
        if (text) {
          sendClipboard(text)
          return
        }
      } catch {
        // Fall back to an explicit paste target when permission is unavailable.
      }
    }
    setClipboardOpen(true)
  }

  const handleRemoteClipboard = useCallback((text: string) => {
    if (!text) return
    if (text === lastSentClipboardRef.current) {
      lastSentClipboardRef.current = ''
      return
    }
    setRemoteClipboard(text)
    setClipboardNotice('已收到远程复制内容')
  }, [])

  const copyRemoteClipboard = () => {
    if (!remoteClipboard) return
    copyWithToast(remoteClipboard, '远程文本已复制到本机')
    setClipboardNotice('')
    remoteRfbRef.current?.focus()
  }

  return (
    <>
      <div className="remote-browser-help">
        <span><i className={connected ? 'connected' : ''} />{connected ? '远程浏览器已连接' : '正在连接远程浏览器…'}</span>
        <div className="remote-browser-help-actions">
          <span>会话将在 {new Date(session.expires_at * 1000).toLocaleTimeString()} 过期</span>
          {remoteClipboard && <button type="button" className="btn btn-sm" disabled={!connected} title="复制远程浏览器剪贴板内容" onClick={copyRemoteClipboard}><CopyIcon size={13} />复制到本机</button>}
          <button type="button" className="btn btn-sm" disabled={!connected} title="粘贴到远程当前输入框" onClick={() => { if (clipboardOpen) closeClipboard(); else void openClipboard() }}><PasteIcon size={13} />{clipboardOpen ? '收起粘贴框' : '粘贴到浏览器'}</button>
        </div>
      </div>
      <RemoteBrowser sessionId={session.id} onConnected={handleConnected} onDisconnected={handleDisconnected} onRfbChange={handleRfbChange} onClipboard={handleRemoteClipboard} />
      {clipboardOpen && (
        <div className="remote-browser-clipboard">
          <textarea
            ref={clipboardInputRef}
            className="input"
            rows={2}
            autoFocus
            aria-label="要发送到远程浏览器的文本"
            placeholder="在这里按 Ctrl/⌘+V，将立即粘贴到远程当前输入框"
            onPaste={(event) => {
              const text = event.clipboardData.getData('text')
              if (!text) return
              event.preventDefault()
              sendClipboard(text)
            }}
            onKeyDown={(event) => {
              if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
                event.preventDefault()
                sendClipboard()
              }
            }}
          />
          <div className="remote-browser-clipboard-actions">
            <span className="small">直接粘贴会立即发送；手动输入后可按 Ctrl/⌘+Enter。</span>
            <button type="button" className="btn btn-sm" onClick={closeClipboard}>取消</button>
            <button type="button" className="btn btn-primary btn-sm" onClick={() => sendClipboard()}><PasteIcon size={13} />粘贴到当前输入框</button>
          </div>
        </div>
      )}
      {clipboardNotice && <p className="remote-browser-clipboard-notice small">{clipboardNotice}</p>}
      {error && <p className="browser-login-error remote-browser-error" role="alert">{error}</p>}
    </>
  )
}

function RemoteBrowser({ sessionId, onConnected, onDisconnected, onRfbChange, onClipboard }: { sessionId: string; onConnected: () => void; onDisconnected: (message?: string) => void; onRfbChange: (rfb: RFB | null) => void; onClipboard: (text: string) => void }) {
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
    rfb.addEventListener('clipboard', (event) => {
      if (!disposed) onClipboard(event.detail.text)
    })
    rfb.focus()
    return () => {
      disposed = true
      onRfbChange(null)
      rfb.disconnect()
      target.replaceChildren()
    }
  }, [sessionId, onConnected, onDisconnected, onRfbChange, onClipboard])

  return <div className="remote-browser-screen" ref={targetRef} />
}
