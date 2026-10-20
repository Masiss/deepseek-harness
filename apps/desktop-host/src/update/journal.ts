/**
 * Append-only evidence for one update attempt.
 *
 * Records carry fixed milestones and whitelisted fields only: no URLs, error
 * text, or request data, so the file can be attached to a support report. Each
 * append is flushed before the caller continues, which is what makes a crash
 * between two milestones visible instead of silent.
 *
 * @module @deepseek-ai/dsh-desktop-host/update/journal
 */

import { randomUUID } from 'node:crypto'
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { isAbsolute, join } from 'node:path'

/** Milestones of one attempt, in the order the service passes them. */
export type DesktopUpdateMilestone = 'started' | 'check' | 'available' | 'download'
  | 'verify' | 'ready' | 'install' | 'installed' | 'failed'

/** Fields a record may carry, all selected by the caller from fixed values. */
export interface DesktopUpdateJournalFields {
  /** Version the attempt targets, once a manifest named one. */
  readonly targetVersion?: string
  /** Integer download progress, only meaningful on `download`. */
  readonly percent?: number
  /** Fixed failure classification, only on `failed`. */
  readonly failure?: string
}

/** One attempt's JSONL evidence file. */
export class DesktopUpdateJournal {
  /** Absolute path of the file this process writes. */
  readonly path: string
  private sequence = 0

  /**
   * @param directory - Absolute evidence directory, retained across installs.
   * @param version - Installed application version at process start.
   */
  constructor(directory: string, private readonly version: string) {
    if (!isAbsolute(directory)) throw new Error('desktop update journal: directory must be absolute')
    mkdirSync(directory, { recursive: true, mode: 0o700 })
    this.path = join(directory, `${Date.now()}-${randomUUID()}.jsonl`)
    writeFileSync(this.path, '', { flag: 'wx', mode: 0o600, flush: true })
    this.milestone('started')
  }

  /**
   * Append one milestone with its whitelisted fields.
   * @param milestone - Fixed step the service reached.
   * @param fields - Optional bounded fields; unknown values are the caller's defect.
   * @returns Nothing after the record is flushed.
   */
  milestone(milestone: DesktopUpdateMilestone, fields: DesktopUpdateJournalFields = {}): void {
    appendFileSync(this.path, `${JSON.stringify({
      schemaVersion: 1,
      sequence: this.sequence++,
      time: new Date().toISOString(),
      pid: process.pid,
      version: this.version,
      milestone,
      ...(fields.targetVersion === undefined ? {} : { targetVersion: fields.targetVersion }),
      ...(fields.percent === undefined ? {} : { percent: Math.floor(fields.percent) }),
      ...(fields.failure === undefined ? {} : { failure: fields.failure }),
    })}\n`, { flush: true })
  }
}
