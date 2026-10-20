/** General Settings row for the on-demand Office component. */
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { ObservableSnapshot } from '@deepseek-ai/dsh-client-store'
import type { InjectFace, PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import css from './OfficeComponentRow.module.css'
import type { DesktopOfficeFailureKind, DesktopOfficePresentation, DesktopOfficeView } from '../types.ts'

/** Carrier capabilities supplied by the settings owner. */
export interface OfficeComponentRowInjected {
  /** False in browsers and on carriers that ship Office in the bundle. */
  readonly available: boolean
  readonly hooks: { office: ObservableSnapshot<DesktopOfficeView> }
  /** Start the download and install. */
  install(): void
  /** Relaunch so an installed component becomes active. */
  restart(): void
}

/** True while the Host owns the install, whether or not the store has seen progress yet. */
function installing(phase: DesktopOfficePresentation['phase'], busy: boolean): boolean {
  return busy || phase === 'downloading' || phase === 'verifying' || phase === 'installing'
}

/**
 * Render Office availability, install progress, and the one action the state allows.
 * @param props - carrier capabilities, shared status, and localized copy.
 * @returns the General Settings row, or nothing on a carrier without components.
 */
export function OfficeComponentRow({ available, useOffice, install, restart, t }:
  PropsLocale<'settings'> & InjectFace<OfficeComponentRowInjected>) {
  const { presentation: state, failed, busy } = useOffice(value => value)
  if (!available) return null
  const phase = state?.phase ?? 'idle'
  const working = installing(phase, busy)
  const installed = state?.installed === true
  const active = state?.active === true
  const version = state?.version
  const error = !working && (failed || phase === 'error')
  const percent = state?.percent ?? 0
  const failure: DesktopOfficeFailureKind = state?.failure ?? 'install'
  const failures: Readonly<Record<DesktopOfficeFailureKind, string>> = {
    feed: t('office.error.generic'),
    'feed-network': t('office.error.network'),
    download: t('office.error.generic'),
    'download-network': t('office.error.network'),
    integrity: t('office.error.integrity'),
    archive: t('office.error.integrity'),
    python: t('office.error.python'),
    install: t('office.error.generic'),
    unavailable: t('office.error.generic'),
  }
  const description = installed
    ? version === undefined ? t('office.installed') : t('office.installedVersion', { version })
    : t('office.description')

  /** The single action this state offers, or a static label once nothing is left to do. */
  function action() {
    if (working) {
      const label = phase === 'downloading' ? t('office.installing', { percent })
        : phase === 'verifying' ? t('office.verifying')
          : t('office.preparing')
      return <Button variant="outline" disabled>{label}</Button>
    }
    if (error) return <Button variant="outline" onClick={install}>{t('office.retry')}</Button>
    if (installed && !active) return <Button variant="primary" onClick={restart}>{t('office.restart')}</Button>
    // The description already reports the installed component.
    if (installed) return null
    return <Button variant="primary" onClick={install}>{t('office.install')}</Button>
  }

  return <div className={css.row}>
    <div>
      <div className={css.title}>{t('office.title')}</div>
      <div className={css.description}>{description}</div>
      {error && <div className={css.error} role="alert">
        {failed ? t('office.error.generic') : failures[failure]}
      </div>}
    </div>
    {action()}
  </div>
}
