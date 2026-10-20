/** Client-owned observation of the optional Desktop Office component API. */
import { createSnapshotStore } from '@deepseek-ai/dsh-client-store'
import type { DesktopOfficeBridge, DesktopOfficeView } from '../types.ts'

/** Owns one bridge subscription and the one install the shell is running. */
export class DesktopOfficeSource {
  /** Framework-observed component status. */
  readonly store = createSnapshotStore<DesktopOfficeView>({ failed: false, busy: false })
  private live = true
  private received = false
  private readonly unsubscribe: (() => void) | undefined

  /** @param bridge - Optional Desktop carrier API, absent in browsers and on the Electron lane. */
  constructor(private readonly bridge: DesktopOfficeBridge | undefined) {
    this.unsubscribe = bridge?.subscribe((presentation) => {
      if (!this.live) return
      this.received = true
      this.store.set({ ...this.store.getSnapshot(), presentation, failed: false })
    })
    void bridge?.status().then((presentation) => {
      if (this.live && !this.received) this.store.set({ ...this.store.getSnapshot(), presentation })
    }, () => {
      if (this.live && !this.received) this.store.set({ ...this.store.getSnapshot(), failed: true })
    })
  }

  /** Whether this carrier can install the component at all. */
  get available(): boolean {
    return this.bridge !== undefined
  }

  /** Start the install; progress arrives over the subscription. */
  install(): void {
    if (!this.live || this.bridge === undefined) return
    const state = this.store.getSnapshot()
    if (state.busy) return
    this.store.set({ ...state, busy: true, failed: false })
    void this.bridge.install().then((presentation) => {
      if (this.live) this.store.set({ ...this.store.getSnapshot(), presentation, busy: false, failed: false })
    }, () => {
      if (this.live) this.store.set({ ...this.store.getSnapshot(), busy: false, failed: true })
    })
  }

  /** Ask the shell to relaunch so a finished install becomes active. */
  restart(): void {
    if (!this.live || this.bridge === undefined) return
    void this.bridge.restart().catch(() => {
      if (this.live) this.store.set({ ...this.store.getSnapshot(), failed: true })
    })
  }

  /** Detach the carrier and ignore any pending status or action completion. */
  dispose(): void {
    this.live = false
    this.unsubscribe?.()
  }
}
