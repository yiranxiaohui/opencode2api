import { useEffect, useState } from 'react'
import { apiStatus } from '../api/client'

export type Phase = 'boot' | 'setup' | 'logged_out' | 'logged_in' | 'error'

export function useSession() {
  const [phase, setPhase] = useState<Phase>('boot')
  const [error, setError] = useState('')

  const boot = () => {
    setError('')
    apiStatus()
      .then((s) => {
        setPhase(s.installed ? (s.logged_in ? 'logged_in' : 'logged_out') : 'setup')
      })
      .catch(() => setPhase('error'))
  }

  useEffect(() => {
    boot()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return { phase, setPhase, boot, error, setError }
}
