import { useState } from 'react'
import { keysApi } from '../api/keys'
import type { ExportPayload } from '../api/types'
import { toast } from '../lib/toast'
import { DownloadIcon, UploadIcon, XIcon } from './icons'

interface Props {
  onImport: (payload: ExportPayload | unknown[]) => void
}

export default function ImportExportBar({ onImport }: Props) {
  const [importing, setImporting] = useState(false)
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)

  const doExport = async () => {
    try {
      const payload = await keysApi.exportAll()
      const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `opencode2api-export-${new Date().toISOString().slice(0, 10)}.json`
      a.click()
      URL.revokeObjectURL(url)
      toast(`已导出 ${payload.items.length} 条（含明文 Key，注意保管）`, 'ok')
    } catch (e) {
      toast(e instanceof Error ? e.message : '导出失败', 'err')
    }
  }

  const doImport = async () => {
    let payload: ExportPayload | unknown[]
    try {
      payload = JSON.parse(text)
      const okShape =
        Array.isArray(payload) ||
        (typeof payload === 'object' && payload !== null && Array.isArray((payload as { items?: unknown }).items))
      if (!okShape) throw new Error('bad shape')
    } catch {
      toast('JSON 格式不正确', 'err')
      return
    }
    setBusy(true)
    try {
      await onImport(payload)
      setImporting(false)
      setText('')
    } catch (e) {
      toast(e instanceof Error ? e.message : '导入失败', 'err')
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <button className="btn" onClick={doExport} title="导出为 JSON（含明文 Key）">
        <DownloadIcon size={13} /> 导出
      </button>
      <button className="btn" onClick={() => setImporting(true)} title="从 JSON 导入">
        <UploadIcon size={13} /> 导入
      </button>

      {importing && (
        <div className="modal-overlay" onClick={() => !busy && setImporting(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>导入账号</h3>
              <button className="btn btn-ghost btn-sm" onClick={() => setImporting(false)}>
                <XIcon size={15} />
              </button>
            </div>
            <div className="modal-body">
              <div className="field">
                <label>JSON（含 name / api_key）</label>
                <textarea
                  className="input imp-ta"
                  placeholder='{"proxies":[],"items":[{"name":"主账号","api_key":"sk-…","tags":["备用"]}]}'
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                />
                <span className="hint">
                  支持新格式（{'{ proxies, items }'}，代理按名称自动复用）与旧格式数组；Base URL 会被忽略并固定使用 OpenCode 官方地址
                </span>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setImporting(false)} disabled={busy}>
                取消
              </button>
              <button className="btn btn-primary" onClick={doImport} disabled={busy || !text.trim()}>
                {busy ? '导入中…' : '导入'}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  )
}
