import { useState } from 'react'
import type { ProxyRecord, ProxyTestKind, ProxyTestResult } from '../api/types'
import { BoltIcon, EditIcon, GlobeIcon, TrashIcon } from './icons'

interface Props {
  proxies: ProxyRecord[]
  onTest: (id: string, kind: ProxyTestKind) => Promise<ProxyTestResult>
  onEdit: (p: ProxyRecord) => void
  onDelete: (p: ProxyRecord) => void
}

interface TestState {
  running: ProxyTestKind | null
  result: ProxyTestResult | null
  error: string | null
}

export default function ProxyList({ proxies, onTest, onEdit, onDelete }: Props) {
  const [tests, setTests] = useState<Record<string, TestState>>({})

  const testProxy = async (proxy: ProxyRecord, kind: ProxyTestKind) => {
    setTests((current) => ({
      ...current,
      [proxy.id]: { running: kind, result: current[proxy.id]?.result ?? null, error: null },
    }))
    try {
      const result = await onTest(proxy.id, kind)
      setTests((current) => ({ ...current, [proxy.id]: { running: null, result, error: null } }))
    } catch (cause) {
      setTests((current) => ({
        ...current,
        [proxy.id]: {
          running: null,
          result: null,
          error: cause instanceof Error ? cause.message : '测试请求失败',
        },
      }))
    }
  }

  const testLabel = (state: TestState | undefined) => {
    if (!state) return null
    if (state.running) return `${state.running.toUpperCase()} 测试中...`
    if (state.result) {
      const method = state.result.kind.toUpperCase()
      const latency = state.result.latency_ms === null ? '--' : `${state.result.latency_ms}ms`
      return `${method} ${latency}${state.result.status ? ` · ${state.result.status}` : ''}`
    }
    return '测试失败'
  }

  return (
    <div className="panel row-list">
      {proxies.length === 0 && (
        <div className="empty">
          <div className="big">🌐</div>
          <p>还没有任何代理，先新增一个转发代理</p>
        </div>
      )}
      {proxies.map((p) => (
        <div className="key-row proxy-row" key={p.id} style={{ cursor: 'default' }}>
          <span className="led ok" title="已配置" />
          <div className="key-name">
            <span className="nm">{p.name}</span>
            {tests[p.id] && (
              <span
                className={`proxy-test-status ${tests[p.id].running ? 'running' : tests[p.id].result?.ok ? 'ok' : 'err'}`}
                title={tests[p.id].result?.error ?? tests[p.id].error ?? undefined}
              >
                {testLabel(tests[p.id])}
              </span>
            )}
          </div>
          <div className="key-url">
            <GlobeIcon size={13} style={{ verticalAlign: '-2px', marginRight: 5 }} />
            <span className="mono">{p.url}</span>
          </div>
          <div className="tags" />
          <div className="meta-num">
            {new Date(p.created_at * 1000).toLocaleDateString()}
          </div>
          <div className="row-actions proxy-test-actions" onClick={(e) => e.stopPropagation()}>
            <button
              className="btn btn-sm"
              title="测试 TCP 连接延迟"
              aria-label={`测试 ${p.name} 的 TCP 连接`}
              disabled={tests[p.id]?.running !== null && tests[p.id]?.running !== undefined}
              onClick={() => void testProxy(p, 'tcp')}
            >
              <BoltIcon size={13} /> TCP
            </button>
            <button
              className="btn btn-sm"
              title="测试 HTTP 请求延迟"
              aria-label={`测试 ${p.name} 的 HTTP 请求`}
              disabled={tests[p.id]?.running !== null && tests[p.id]?.running !== undefined}
              onClick={() => void testProxy(p, 'http')}
            >
              <GlobeIcon size={13} /> HTTP
            </button>
            <button className="btn btn-sm" title="编辑" onClick={() => onEdit(p)}>
              <EditIcon size={13} />
            </button>
            <button className="btn btn-sm btn-danger" title="删除" onClick={() => onDelete(p)}>
              <TrashIcon size={13} />
            </button>
          </div>
        </div>
      ))}
    </div>
  )
}
