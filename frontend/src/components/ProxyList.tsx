import type { ProxyRecord } from '../api/types'
import { EditIcon, GlobeIcon, TrashIcon } from './icons'

interface Props {
  proxies: ProxyRecord[]
  onEdit: (p: ProxyRecord) => void
  onDelete: (p: ProxyRecord) => void
}

export default function ProxyList({ proxies, onEdit, onDelete }: Props) {
  return (
    <div className="panel row-list">
      {proxies.length === 0 && (
        <div className="empty">
          <div className="big">🌐</div>
          <p>还没有任何代理，先新增一个转发代理</p>
        </div>
      )}
      {proxies.map((p) => (
        <div className="key-row" key={p.id} style={{ cursor: 'default' }}>
          <span className="led ok" title="已配置" />
          <div className="key-name">
            <span className="nm">{p.name}</span>
          </div>
          <div className="key-url">
            <GlobeIcon size={13} style={{ verticalAlign: '-2px', marginRight: 5 }} />
            <span className="mono">{p.url}</span>
          </div>
          <div className="tags" />
          <div className="meta-num">
            {new Date(p.created_at * 1000).toLocaleDateString()}
          </div>
          <div className="row-actions" onClick={(e) => e.stopPropagation()}>
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
