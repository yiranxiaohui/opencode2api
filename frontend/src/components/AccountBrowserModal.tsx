import { useEffect, useState } from 'react'
import { browserLoginApi } from '../api/keys'
import type { BrowserLoginSession } from '../api/types'
import { XIcon } from './icons'
import RemoteBrowserDesktop from './RemoteBrowserDesktop'

interface Props {
  accountName: string
  session: BrowserLoginSession
  onClose: () => void
}

export default function AccountBrowserModal({ accountName, session, onClose }: Props) {
  const [closing, setClosing] = useState(false)

  useEffect(() => () => {
    void browserLoginApi.stop(session.id).catch(() => undefined)
  }, [session.id])

  const close = async () => {
    if (closing) return
    setClosing(true)
    await browserLoginApi.stop(session.id).catch(() => undefined)
    onClose()
  }

  return (
    <div className="modal-overlay remote-browser-overlay">
      <div className="modal remote-browser-modal">
        <div className="modal-head">
          <h3>订阅 OpenCode Go · {accountName}</h3>
          <button type="button" className="btn btn-ghost btn-sm" disabled={closing} onClick={() => void close()} aria-label="关闭">
            <XIcon size={16} />
          </button>
        </div>
        <RemoteBrowserDesktop session={session} />
        <div className="modal-foot remote-browser-foot">
          <span className="small">已注入该账号 Cookie，并使用账号绑定的出口代理。完成订阅后关闭窗口，再刷新套餐与额度。</span>
          <button type="button" className="btn" disabled={closing} onClick={() => void close()}>{closing ? '正在关闭…' : '关闭浏览器'}</button>
        </div>
      </div>
    </div>
  )
}
