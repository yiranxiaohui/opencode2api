declare module '@novnc/novnc' {
  export interface RFBEventMap {
    connect: Event
    disconnect: CustomEvent<{ clean: boolean }>
    securityfailure: CustomEvent<{ status: number; reason: string }>
  }

  export default class RFB {
    constructor(target: HTMLElement, url: string, options?: { credentials?: { password?: string } })
    scaleViewport: boolean
    resizeSession: boolean
    clipViewport: boolean
    focusOnClick: boolean
    qualityLevel: number
    compressionLevel: number
    addEventListener<K extends keyof RFBEventMap>(type: K, listener: (event: RFBEventMap[K]) => void): void
    clipboardPasteFrom(text: string): void
    disconnect(): void
    focus(): void
  }
}
