/** Type-only Desktop carrier declarations shared by the settings UI and the carrier checks. */

/** Classified failure copy selected by the Web locale without exposing raw updater diagnostics. */
export type DesktopUpdateFailureKind =
  | 'check'
  | 'check-network'
  | 'download'
  | 'download-network'
  | 'install'
  | 'install-network'
  | 'stop-failed'
  | 'tasks-changed'
  | 'tasks-unavailable'

/** Public semantic preload fields consumed by the settings row. */
export interface DesktopUpdatePresentation {
  readonly phase: 'idle' | 'checking' | 'available' | 'downloading' | 'verifying' | 'installing' | 'ready' | 'error'
  readonly version?: string
  readonly percent?: number
  readonly failure?: DesktopUpdateFailureKind
}

/** Optional carrier API; it cannot select artifacts or authorize installation. */
export interface DesktopUpdateBridge {
  status(): Promise<DesktopUpdatePresentation>
  open(): Promise<void>
  subscribe(listener: (state: DesktopUpdatePresentation) => void): () => void
}

/** Shared carrier status for the account row and collapsed sidebar badge. */
export interface DesktopUpdateView {
  readonly presentation?: DesktopUpdatePresentation
  readonly failed: boolean
  readonly opening: boolean
}

/** Classified install failure selected by the Web locale without exposing Host diagnostics. */
export type DesktopOfficeFailureKind = 'feed' | 'feed-network' | 'download' | 'download-network'
  | 'integrity' | 'archive' | 'python' | 'install' | 'unavailable'

/** Public semantic component fields consumed by the settings row. */
export interface DesktopOfficePresentation {
  readonly id: 'office'
  readonly installed: boolean
  readonly active: boolean
  readonly phase: 'idle' | 'downloading' | 'verifying' | 'installing' | 'ready' | 'error'
  readonly version?: string
  readonly percent?: number
  readonly failure?: DesktopOfficeFailureKind
}

/** Optional carrier API; it cannot choose an artifact or trust an unverified archive. */
export interface DesktopOfficeBridge {
  status(): Promise<DesktopOfficePresentation>
  install(): Promise<DesktopOfficePresentation>
  restart(): Promise<void>
  subscribe(listener: (state: DesktopOfficePresentation) => void): () => void
}

/** Shared carrier status for the General Settings row. */
export interface DesktopOfficeView {
  readonly presentation?: DesktopOfficePresentation
  readonly failed: boolean
  readonly busy: boolean
}
