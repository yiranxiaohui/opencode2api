import { useCallback, useEffect, useState } from 'react'
import { keysApi } from '../api/keys'
import type { InviteReward, InviteRewardsResult, KeySummary } from '../api/types'
import { toast } from '../lib/toast'
import { GiftIcon, RefreshIcon, XIcon } from './icons'

interface Props {
  account: KeySummary
  onClose: () => void
}

const money = (amountCents: number) =>
  new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  }).format(amountCents / 100)

const rewardDate = (value: string | null) => {
  if (!value) return '—'
  const numeric = /^\d+$/.test(value) ? Number(value) : Number.NaN
  const date = Number.isFinite(numeric)
    ? new Date(numeric < 10_000_000_000 ? numeric * 1_000 : numeric)
    : new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString('zh-CN')
}

const description = (reward: InviteReward) => {
  if (reward.source === 'inviter') return reward.email ? `已邀请 ${reward.email}` : '邀请好友奖励'
  if (reward.source === 'invitee') return reward.email ? `受 ${reward.email} 邀请` : '受邀注册奖励'
  return reward.email || reward.source
}

const statusLabel = (reward: InviteReward) => {
  if (reward.status === 'applied') return '奖励已使用'
  if (reward.claimable) return '待使用'
  return reward.status
}

export default function InviteRewardsModal({ account, onClose }: Props) {
  const [result, setResult] = useState<InviteRewardsResult | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [claimingId, setClaimingId] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      setResult(await keysApi.inviteRewards(account.id))
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '邀请奖励查询失败')
    } finally {
      setLoading(false)
    }
  }, [account.id])

  useEffect(() => {
    void load()
  }, [load])

  const claim = async (reward: InviteReward) => {
    setClaimingId(reward.id)
    try {
      const claimed = await keysApi.claimInviteReward(account.id, reward.id)
      setResult((current) => current && ({
        ...current,
        rewards: current.rewards.map((item) => item.id === reward.id
          ? { ...item, status: 'applied', claimable: false }
          : item),
      }))
      toast(`${account.name} 已使用 ${money(claimed.amount_cents)} 邀请奖励`, 'ok')
    } catch (cause) {
      toast(cause instanceof Error ? cause.message : '邀请奖励领取失败', 'err')
    } finally {
      setClaimingId(null)
    }
  }

  const rewards = result?.rewards ?? []
  const pendingCount = rewards.filter((reward) => reward.claimable).length

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal invite-reward-modal" role="dialog" aria-modal="true" aria-labelledby="invite-reward-title" onClick={(event) => event.stopPropagation()}>
        <div className="modal-head">
          <div className="invite-reward-heading">
            <h3 id="invite-reward-title"><GiftIcon size={16} /> 邀请奖励</h3>
            <span>{account.name}</span>
          </div>
          <button className="btn btn-ghost btn-sm" type="button" onClick={onClose} aria-label="关闭">
            <XIcon size={15} />
          </button>
        </div>
        <div className="modal-body">
          <div className="invite-reward-summary">
            <span>邀请奖励将应用到该账号的 Go 用量。</span>
            {!loading && !error && <strong>{rewards.length} 条奖励 · {pendingCount} 条待使用</strong>}
            <button className="btn btn-sm" type="button" disabled={loading || claimingId !== null} onClick={() => void load()}>
              <RefreshIcon size={12} className={loading ? 'spin' : undefined} /> {loading ? '查询中…' : '刷新'}
            </button>
          </div>

          {error && (
            <div className="invite-reward-error">
              <p className="auth-err">{error}</p>
              <button className="btn btn-sm" type="button" onClick={() => void load()}>重试</button>
            </div>
          )}

          {!error && loading && <div className="invite-reward-loading"><RefreshIcon size={18} className="spin" /> 正在读取 OpenCode 邀请奖励…</div>}

          {!error && !loading && rewards.length === 0 && (
            <div className="invite-reward-empty">当前账号还没有邀请奖励</div>
          )}

          {!error && !loading && rewards.length > 0 && (
            <div className="invite-reward-table">
              <div className="invite-reward-row invite-reward-table-head">
                <span>奖励</span><span>描述</span><span>日期</span><span>状态</span>
              </div>
              {rewards.map((reward) => (
                <div className="invite-reward-row" key={reward.id}>
                  <strong className="invite-reward-amount">{money(reward.amount_cents)}</strong>
                  <span className="invite-reward-description" title={description(reward)}>{description(reward)}</span>
                  <span className="invite-reward-date">{rewardDate(reward.created_at)}</span>
                  <span className="invite-reward-action">
                    {reward.claimable ? (
                      <button className="btn btn-primary btn-sm" type="button" disabled={claimingId !== null} onClick={() => void claim(reward)}>
                        {claimingId === reward.id ? '使用中…' : '使用奖励'}
                      </button>
                    ) : (
                      <span className={`invite-reward-status ${reward.status === 'applied' ? 'applied' : ''}`}>{statusLabel(reward)}</span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
