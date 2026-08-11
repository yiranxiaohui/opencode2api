import { useEffect, useState } from 'react'
import { setLockedHandler } from './api/client'
import type { ClientApiKey, KeyInput, KeySummary, ProxyRecord } from './api/types'
import AuthScreen from './components/AuthScreen'
import ImportExportBar from './components/ImportExportBar'
import KeyDetail from './components/KeyDetail'
import CookieImportModal from './components/CookieImportModal'
import KeyFormModal from './components/KeyFormModal'
import KeyList from './components/KeyList'
import Logs from './components/Logs'
import ClientKeys from './components/ClientKeys'
import ProxyFormModal from './components/ProxyFormModal'
import ProxyList from './components/ProxyList'
import Settings from './components/Settings'
import ToastHost from './components/ToastHost'
import { ActivityIcon, GatewayIcon, GearIcon, GlobeIcon, KeyIcon, PlusIcon, VaultIcon } from './components/icons'
import { useKeys } from './hooks/useKeys'
import { useProxies } from './hooks/useProxies'
import { useSession } from './hooks/useSession'
import { clientKeysApi } from './api/keys'
import { toast } from './lib/toast'

type Tab = 'keys' | 'logs' | 'proxies' | 'client-keys' | 'settings'

export default function App() {
  const session = useSession()
  const [tab, setTab] = useState<Tab>('keys')
  const [formOpen, setFormOpen] = useState(false)
  const [cookieImportOpen, setCookieImportOpen] = useState(false)
  const [editing, setEditing] = useState<KeySummary | null>(null)
  const [detail, setDetail] = useState<KeySummary | null>(null)
  const [pendingDelete, setPendingDelete] = useState<KeySummary | null>(null)
  const [busy, setBusy] = useState(false)
  const [proxyFormOpen, setProxyFormOpen] = useState(false)
  const [editingProxy, setEditingProxy] = useState<ProxyRecord | null>(null)
  const [pendingProxyDelete, setPendingProxyDelete] = useState<ProxyRecord | null>(null)

  const { query, createKey, updateKey, deleteKey, setEnabled, testKey, importItems, importCookie } = useKeys(
    session.phase === 'unlocked',
  )
  const { query: proxiesQuery, createProxy, updateProxy, deleteProxy } = useProxies(
    session.phase === 'unlocked',
  )

  useEffect(() => {
    setLockedHandler(() => {
      session.boot()
      setTab('keys')
      setDetail(null)
      setFormOpen(false)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const keys = query.data ?? []
  const proxies = proxiesQuery.data ?? []
  const [clientKeys, setClientKeys] = useState<ClientApiKey[]>([])
  useEffect(() => {
    if (session.phase !== 'unlocked') return
    let live = true
    clientKeysApi
      .list()
      .then((items) => { if (live) setClientKeys(items) })
      .catch(() => { /* filters still work without the list */ })
    return () => { live = false }
  }, [session.phase])

  const handleSave = (input: KeyInput) => {
    setBusy(true)
    const done = () => setBusy(false)
    if (editing) {
      updateKey.mutate({ id: editing.id, input }, { onSettled: done })
    } else {
      createKey.mutate(input, { onSettled: done })
    }
    setFormOpen(false)
    setEditing(null)
  }

  const handleDelete = () => {
    if (!pendingDelete) return
    deleteKey.mutate(pendingDelete.id, { onSettled: () => setBusy(false) })
    setDetail(null)
    setPendingDelete(null)
  }

  const quickTest = (k: KeySummary) => {
    testKey.mutate(k.id, {
      onSuccess: (r) => toast(r.ok ? `${k.name} 连通正常 · ${r.latency_ms}ms` : r.error ?? '连接失败', r.ok ? 'ok' : 'err'),
      onError: (e) => toast(e instanceof Error ? e.message : '测试失败', 'err'),
    })
  }

  const handleProxySave = (input: { name: string; url: string }) => {
    setBusy(true)
    const done = () => setBusy(false)
    if (editingProxy) {
      updateProxy.mutate({ id: editingProxy.id, input }, { onSettled: done })
    } else {
      createProxy.mutate(input, { onSettled: done })
    }
    setProxyFormOpen(false)
    setEditingProxy(null)
  }

  const handleProxyDelete = () => {
    if (!pendingProxyDelete) return
    deleteProxy.mutate(pendingProxyDelete.id, { onSettled: () => setBusy(false) })
    setPendingProxyDelete(null)
  }

  const renderTab = () => {
    if (tab === 'logs') return <Logs keys={keys} clientKeys={clientKeys} />
    if (tab === 'client-keys') return <ClientKeys onKeysChange={setClientKeys} />
    if (tab === 'settings') return <Settings />
    if (tab === 'proxies')
      return (
        <>
          <div className="toolbar">
            <div className="topbar-title">
              代理池
              <span className="count">{proxies.length} 条</span>
            </div>
            <div className="grow" />
            <button
              className="btn btn-primary"
              onClick={() => {
                setEditingProxy(null)
                setProxyFormOpen(true)
              }}
            >
              <PlusIcon size={13} /> 新增代理
            </button>
          </div>
          <ProxyList
            proxies={proxies}
            onEdit={(p) => {
              setEditingProxy(p)
              setProxyFormOpen(true)
            }}
            onDelete={setPendingProxyDelete}
          />
        </>
      )
    return (
      <>
        <div className="toolbar">
          <div className="topbar-title">
            账号
            <span className="count">{keys.length} 条</span>
          </div>
          <div className="grow" />
          <ImportExportBar onImport={(items) => importItems.mutate(items)} />
          <button className="btn" onClick={() => setCookieImportOpen(true)}>Cookie 导入</button>
          <button
            className="btn btn-primary"
            onClick={() => {
              setEditing(null)
              setFormOpen(true)
            }}
          >
            <PlusIcon size={13} /> 新增账号
          </button>
        </div>

        <KeyList
          keys={keys}
          selectedId={detail?.id ?? null}
          onOpen={setDetail}
          onTest={quickTest}
          onEdit={(k) => {
            setEditing(k)
            setFormOpen(true)
          }}
          onDelete={setPendingDelete}
          onSetEnabled={(id, enabled) => setEnabled.mutate({ id, enabled })}
        />
      </>
    )
  }

  if (session.phase === 'boot') return <BootScreen />
  if (session.phase === 'setup') return <AuthScreen mode="setup" onDone={() => session.setPhase('unlocked')} />
  if (session.phase === 'locked') return <AuthScreen mode="unlock" onDone={() => session.setPhase('unlocked')} />
  if (session.phase === 'error')
    return <BootScreen error="无法连接后端，请确认 cargo run 已在 127.0.0.1:8787 启动" />

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">
            <GatewayIcon size={20} />
          </div>
          <div>
            <div className="brand-name">
              opencode2api
            </div>
            <div className="brand-sub">API Gateway</div>
          </div>
        </div>
        <button className={`nav-item ${tab === 'keys' ? 'active' : ''}`} onClick={() => setTab('keys')}>
          <KeyIcon /> 账号管理
        </button>
        <button className={`nav-item ${tab === 'logs' ? 'active' : ''}`} onClick={() => setTab('logs')}>
          <ActivityIcon /> 请求日志
        </button>
        <button className={`nav-item ${tab === 'proxies' ? 'active' : ''}`} onClick={() => setTab('proxies')}>
          <GlobeIcon /> 代理池
        </button>
        <button className={`nav-item ${tab === 'client-keys' ? 'active' : ''}`} onClick={() => setTab('client-keys')}>
          <VaultIcon /> 密钥管理
        </button>
        <button className={`nav-item ${tab === 'settings' ? 'active' : ''}`} onClick={() => setTab('settings')}>
          <GearIcon /> 设置
        </button>
      </aside>

      <main className="main">
        <div className="topbar">
          <div className="topbar-title">
            {tab === 'keys' && '账号管理'}
            {tab === 'logs' && '请求日志'}
            {tab === 'proxies' && '代理池'}
            {tab === 'client-keys' && '密钥管理'}
            {tab === 'settings' && '设置'}
          </div>
          <div className="topbar-spacer" />
        </div>
        <div className="content">{renderTab()}</div>
      </main>

      {formOpen && (
        <KeyFormModal
          initial={editing ?? undefined}
          proxies={proxies}
          busy={busy}
          onClose={() => {
            setFormOpen(false)
            setEditing(null)
          }}
          onSave={handleSave}
        />
      )}
      {cookieImportOpen && <CookieImportModal proxies={proxies} busy={busy} onClose={() => setCookieImportOpen(false)} onSave={(input) => { setBusy(true); importCookie.mutate(input, { onSettled: () => setBusy(false), onSuccess: () => setCookieImportOpen(false) }) }} />}

      {proxyFormOpen && (
        <ProxyFormModal
          initial={editingProxy ?? undefined}
          busy={busy}
          onClose={() => {
            setProxyFormOpen(false)
            setEditingProxy(null)
          }}
          onSave={handleProxySave}
        />
      )}

      {detail && (
        <KeyDetail
          summary={detail}
          onClose={() => setDetail(null)}
          onEdit={(k) => {
            setEditing(k)
            setFormOpen(true)
          }}
          onDelete={setPendingDelete}
          onSetEnabled={(id, enabled) => {
            setEnabled.mutate({ id, enabled })
            setDetail(null)
          }}
        />
      )}

      {pendingDelete && (
        <div className="modal-overlay" onClick={() => setPendingDelete(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-body" style={{ paddingTop: 24 }}>
              <h3 style={{ margin: '0 0 8px' }}>删除账号「{pendingDelete.name}」？</h3>
              <p className="small" style={{ margin: 0 }}>
                此操作不可撤销。该账号的 API Key 与模型缓存将被一并移除。
              </p>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setPendingDelete(null)}>
                取消
              </button>
              <button className="btn btn-danger" onClick={handleDelete}>
                确认删除
              </button>
            </div>
          </div>
        </div>
      )}

      {pendingProxyDelete && (
        <div className="modal-overlay" onClick={() => setPendingProxyDelete(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-body" style={{ paddingTop: 24 }}>
              <h3 style={{ margin: '0 0 8px' }}>删除代理「{pendingProxyDelete.name}」？</h3>
              <p className="small" style={{ margin: 0 }}>
                此操作不可撤销。使用该代理的所有账号将恢复为直连。
              </p>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setPendingProxyDelete(null)}>
                取消
              </button>
              <button className="btn btn-danger" onClick={handleProxyDelete}>
                确认删除
              </button>
            </div>
          </div>
        </div>
      )}

      <ToastHost />
    </div>
  )
}

function BootScreen({ error }: { error?: string }) {
  return (
    <div className="auth-wrap">
      <div className="auth-card" style={{ textAlign: 'center' }}>
        <div className="auth-mark" style={{ margin: '0 auto 20px' }}>
          <GatewayIcon size={28} />
        </div>
        {error ? (
          <>
            <h1>无法连接后端</h1>
            <p className="tagline">请确认服务已启动：<span className="mono">cargo run</span></p>
          </>
        ) : (
          <>
            <h1>opencode2api</h1>
            <p className="tagline">正在登录…</p>
          </>
        )}
      </div>
    </div>
  )
}
