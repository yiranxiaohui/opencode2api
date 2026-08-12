interface IconProps {
  size?: number
  style?: React.CSSProperties
  className?: string
}

const base = (p: IconProps) => ({
  width: p.size ?? 16,
  height: p.size ?? 16,
  style: p.style,
  className: p.className,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
})

export const KeyIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M15.5 7.5a3 3 0 1 0-2.12 5.12L7 19v2h3v-2h2v-2h2l1.5-1.5A3 3 0 0 0 15.5 7.5Z" />
    <circle cx="15.5" cy="8.5" r="1" fill="currentColor" stroke="none" />
  </svg>
)

export const VaultIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <rect x="4" y="4" width="16" height="16" rx="3" />
    <circle cx="12" cy="12" r="3.2" />
    <path d="M12 8.8v3.2M10.6 13.6l2-2" />
  </svg>
)

export const GatewayIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })} aria-hidden="true">
    <circle cx="4" cy="6" r="1.5" fill="currentColor" stroke="none" />
    <circle cx="4" cy="18" r="1.5" fill="currentColor" stroke="none" />
    <circle cx="20" cy="6" r="1.5" fill="currentColor" stroke="none" />
    <circle cx="20" cy="18" r="1.5" fill="currentColor" stroke="none" />
    <path d="M5.5 6h3L12 12l3.5-6h3M5.5 18h3l3.5-6 3.5 6h3" />
    <rect x="9.5" y="9.5" width="5" height="5" rx="1.5" fill="var(--panel-2, #141210)" />
  </svg>
)

export const PlayIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <circle cx="12" cy="12" r="9" />
    <path d="M10 8.8v6.4l5.2-3.2L10 8.8Z" fill="currentColor" stroke="none" />
  </svg>
)

export const ChatIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M5 5.5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H10l-5 3v-3a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2Z" />
    <path d="M7.5 10h9M7.5 13.5h5" />
  </svg>
)

export const SendIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="m3 3 18 9-18 9 3.5-9L3 3Z" />
    <path d="M6.5 12H21" />
  </svg>
)

export const StopIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" stroke="none" />
  </svg>
)

export const GearIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2.5l1 3.2 3-.9 1.8 2.7 3.1-.7.9 3-2.4 2.2 1.9 2.7-2.6 1.8.2 3.2-3.1.6-1.5 2.9-3.1-1-2.1 2.4-2.8-1.6-.4-3.2-3.1-.5.7-3.1-2.3-2.3 1.7-2.8-2.6-1.9 2.1-2.5-1.7-2.7 2.9-1.4.1-3.2 3.2-.1z" transform="translate(0 1)" />
  </svg>
)

export const LockIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <rect x="5" y="10" width="14" height="10" rx="2" />
    <path d="M8 10V7a4 4 0 0 1 8 0v3" />
  </svg>
)

export const PlusIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 5v14M5 12h14" />
  </svg>
)

export const SearchIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="m20 20-3.8-3.8" />
  </svg>
)

export const CopyIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M5 15H4.5A1.5 1.5 0 0 1 3 13.5v-9A1.5 1.5 0 0 1 4.5 3h9A1.5 1.5 0 0 1 15 4.5V5" />
  </svg>
)

export const CheckIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="m4.5 12.5 5 5 10-11" />
  </svg>
)

export const XIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M6 6l12 12M18 6L6 18" />
  </svg>
)

export const EditIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M4 20h4l10.5-10.5a2.12 2.12 0 0 0-3-3L5 17v3Z" />
    <path d="m13.5 6.5 3 3" />
  </svg>
)

export const TrashIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12" />
    <path d="M10 11v6M14 11v6" />
  </svg>
)

export const BoltIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M13 2 4.5 13.5H11L9.5 22 19 10h-6.5L13 2Z" />
  </svg>
)

export const PowerIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 2v10" />
    <path d="M6.3 5.7a8 8 0 1 0 11.4 0" />
  </svg>
)

export const DownloadIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 3v12m0 0 5-5m-5 5-5-5" />
    <path d="M4 19h16" />
  </svg>
)

export const UploadIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 21V9m0 0 5 5m-5-5-5 5" />
    <path d="M4 5h16" />
  </svg>
)

export const EyeIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M2 12s3.5-6.5 10-6.5S22 12 22 12s-3.5 6.5-10 6.5S2 12 2 12Z" />
    <circle cx="12" cy="12" r="2.8" />
  </svg>
)

export const EyeOffIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M2 12s3.5-6.5 10-6.5c1.8 0 3.4.4 4.8 1.1M22 12s-3.5 6.5-10 6.5a9.6 9.6 0 0 1-4.8-1.1" />
    <path d="M9.5 9.6a2.8 2.8 0 0 0 3.9 3.9M4 4l16 16" />
  </svg>
)

export const RefreshIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M20 12a8 8 0 1 1-2.34-5.66M20 3v4h-4" />
  </svg>
)

export const ShieldIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 2.5 4.5 5.5v5c0 5 3.2 8.8 7.5 11 4.3-2.2 7.5-6 7.5-11v-5L12 2.5Z" />
  </svg>
)

export const StarIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="m12 3 2.7 5.6 6.1.8-4.5 4.3 1.1 6L12 16.8 6.6 19.7l1.1-6L3.2 9.4l6.1-.8L12 3Z" />
  </svg>
)

export const GlobeIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3c2.5 2.4 3.8 5.5 3.8 9S14.5 18.6 12 21c-2.5-2.4-3.8-5.5-3.8-9S9.5 5.4 12 3Z" />
  </svg>
)

export const ActivityIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M3 12h4l2.5-7 4.5 14 2.5-7h4.5" />
  </svg>
)

export const ModelIcon = ({ size, style, className }: IconProps) => (
  <svg {...base({ size, style, className })}>
    <path d="M12 3 4 7l8 4 8-4-8-4Z" />
    <path d="m4 12 8 4 8-4M4 17l8 4 8-4" />
  </svg>
)
