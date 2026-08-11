import { useEffect, useState } from 'react'
import { apiStatus } from '../api/client'

export type Phase = 'boot' | 'setup' | 'locked' | 'unlocked' | 'error'

export function useSession() {
  const [phase, setPhase] = useState<Phase>('boot')
  const [error, setError] = useState('')

  const boot = () => {
    setError('')
    apiStatus()
      .then((s) => {
        setPhase(s.installed ? (s.unlocked ? 'unlocked' : 'locked') : 'setup')
      })
      .catch(() => setPhase('error'))
  }

  useEffect(() => {
    boot()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return { phase, setPhase, boot, error, setError }
}
