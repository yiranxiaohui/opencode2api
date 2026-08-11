import { useState } from 'react'
import { copyWithToast } from '../lib/toast'
import { CheckIcon, CopyIcon } from './icons'

interface Props {
  text: string
  label?: string
  className?: string
}

export default function CopyButton({ text, label = '复制', className = '' }: Props) {
  const [copied, setCopied] = useState(false)

  return (
    <button
      type="button"
      className={`btn btn-sm ${className}`}
      title={`复制 ${label}`}
      onClick={() => {
        copyWithToast(text, `已复制${label}`)
        setCopied(true)
        setTimeout(() => setCopied(false), 1200)
      }}
    >
      {copied ? <CheckIcon size={13} /> : <CopyIcon size={13} />}
      {copied ? '已复制' : label}
    </button>
  )
}
