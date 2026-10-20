// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { useSyncExternalStore } from 'react'
import { afterEach, expect, it, vi } from 'vitest'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import { OfficeComponentRow } from '../src/client/OfficeComponentRow.tsx'
import { DesktopOfficeSource } from '../src/client/office-component-source.ts'
import { en, zh } from '../src/client/locales.ts'
import type {
  DesktopOfficeBridge, DesktopOfficeFailureKind, DesktopOfficePresentation,
} from '../src/types.ts'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })

type SettingsTranslate = PropsLocale<'settings'>['t']

function translate(dictionary: typeof zh | typeof en): SettingsTranslate {
  const messages: Readonly<Record<string, string>> = dictionary
  return (key, params) => Object.entries(params ?? {})
    .reduce((message, [name, value]) => message.replaceAll(`{${name}}`, String(value)),
      messages[key] ?? key)
}

const idle: DesktopOfficePresentation = { id: 'office', installed: false, active: false, phase: 'idle' }

function fixture(options: { bridge?: boolean; dictionary?: typeof zh | typeof en } = {}) {
  const status = Promise.withResolvers<DesktopOfficePresentation>()
  let listener: ((state: DesktopOfficePresentation) => void) | undefined
  const unsubscribe = vi.fn()
  const install = vi.fn(async () => idle)
  const restart = vi.fn(async () => {})
  const bridge: DesktopOfficeBridge = {
    status: () => status.promise,
    install,
    restart,
    subscribe: (next) => { listener = next; return unsubscribe },
  }
  const source = new DesktopOfficeSource(options.bridge === false ? undefined : bridge)
  const snapshot = () => source.store.getSnapshot()
  const subscribe = (notify: () => void) => source.store.subscribe(notify)

  /** Render the row against the source, with the selector face the slot injects. */
  function Row({ dictionary = options.dictionary ?? zh }: { dictionary?: typeof zh | typeof en }) {
    const state = useSyncExternalStore(subscribe, snapshot)
    return <OfficeComponentRow
      available={source.available}
      useOffice={selector => selector(state)}
      install={() => { source.install() }}
      restart={() => { source.restart() }}
      t={translate(dictionary)} />
  }

  const view = render(<Row />)
  const unmount = view.unmount
  view.unmount = () => { source.dispose(); unmount() }
  return {
    status, install, restart, unsubscribe, view, source, Row,
    emit: async (state: DesktopOfficePresentation) => { await act(async () => { listener?.(state) }) },
  }
}

it('renders nothing when the carrier has no component API', () => {
  const f = fixture({ bridge: false })
  f.source.install()
  f.source.restart()
  f.source.dispose()
  expect(f.view.container.innerHTML).toBe('')
  expect(f.install).not.toHaveBeenCalled()
})

it('offers Install and reports the settled status', async () => {
  const f = fixture()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockResolvedValueOnce({ id: 'office', installed: true, active: false, phase: 'ready', version: '1.2.3' })
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await act(async () => {})
    expect(f.install).toHaveBeenCalledOnce()
    expect(screen.getByText('已安装 1.2.3')).toBeTruthy()
    expect(screen.getByRole('button', { name: '重启以启用' })).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('describes every install stage and disables the action while it runs', async () => {
  const f = fixture()
  try {
    await f.emit({ id: 'office', installed: false, active: false, phase: 'downloading', percent: 42 })
    expect(screen.getByRole('button', { name: '正在安装 42%' }).hasAttribute('disabled')).toBe(true)
    await f.emit({ id: 'office', installed: false, active: false, phase: 'downloading' })
    expect(screen.getByRole('button', { name: '正在安装 0%' })).toBeTruthy()
    await f.emit({ id: 'office', installed: false, active: false, phase: 'verifying' })
    expect(screen.getByRole('button', { name: '正在校验' })).toBeTruthy()
    await f.emit({ id: 'office', installed: false, active: false, phase: 'installing' })
    expect(screen.getByRole('button', { name: '正在准备' })).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('shows a disabled preparing action while the shell install is not yet reporting', async () => {
  const f = fixture()
  const pending = Promise.withResolvers<DesktopOfficePresentation>()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockImplementationOnce(() => pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await act(async () => {})
    expect(screen.getByRole('button', { name: '正在准备' }).hasAttribute('disabled')).toBe(true)
    await act(async () => { pending.resolve({ ...idle, installed: true, active: true, phase: 'ready' }) })
    expect(screen.queryByRole('button')).toBeNull()
    expect(screen.getByText('已安装')).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it.each<[DesktopOfficeFailureKind, string]>([
  ['feed', '安装失败，请重试'],
  ['feed-network', '安装失败，请检查网络连接后重试'],
  ['download', '安装失败，请重试'],
  ['download-network', '安装失败，请检查网络连接后重试'],
  ['integrity', '安装文件已损坏，请重试'],
  ['archive', '安装文件已损坏，请重试'],
  ['python', 'Python 环境未通过校验，请重试'],
  ['install', '安装失败，请重试'],
  ['unavailable', '安装失败，请重试'],
])('maps %s to actionable copy', async (failure, message) => {
  const f = fixture()
  try {
    await f.emit({ id: 'office', installed: false, active: false, phase: 'error', failure })
    expect(screen.getByRole('alert').textContent).toBe(message)
    expect(screen.getByRole('button', { name: '重试' })).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('retries after a bridge failure and clears the error', async () => {
  const f = fixture()
  try {
    await act(async () => { f.status.reject(new Error('bridge unavailable')) })
    expect(screen.getByRole('alert').textContent).toBe('安装失败，请重试')
    f.install.mockResolvedValueOnce({ id: 'office', installed: true, active: true, phase: 'ready' })
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    await act(async () => {})
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByText('已安装')).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('asks the shell to restart when the component is installed but inactive', async () => {
  const f = fixture()
  try {
    await f.emit({ id: 'office', installed: true, active: false, phase: 'ready', version: '1.2.3' })
    fireEvent.click(screen.getByRole('button', { name: '重启以启用' }))
    await act(async () => {})
    expect(f.restart).toHaveBeenCalledOnce()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('renders the same row in English', async () => {
  const f = fixture({ dictionary: en })
  try {
    await f.emit(idle)
    expect(screen.getByText('Office document support')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Install' })).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('keeps the newest event and ignores late carrier answers', async () => {
  const f = fixture()
  try {
    await f.emit({ id: 'office', installed: true, active: true, phase: 'ready' })
    await act(async () => { f.status.resolve(idle) })
    expect(screen.getByText('已安装')).toBeTruthy()
    await act(async () => { f.status.reject(new Error('late')) })
    expect(screen.queryByRole('alert')).toBeNull()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})

it('ignores events, installs, and restarts after disposal', async () => {
  const f = fixture()
  const snapshot = f.source.store.getSnapshot()
  f.view.unmount()
  f.source.install()
  f.source.restart()
  await f.emit({ id: 'office', installed: false, active: false, phase: 'error', failure: 'python' })
  await act(async () => { f.status.resolve(idle) })
  expect(f.source.store.getSnapshot()).toBe(snapshot)
  expect(f.unsubscribe).toHaveBeenCalledOnce()
  expect(f.install).not.toHaveBeenCalled()
  expect(f.restart).not.toHaveBeenCalled()
})

it('coalesces a second install and surfaces a rejected restart', async () => {
  const f = fixture()
  const pending = Promise.withResolvers<DesktopOfficePresentation>()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockImplementationOnce(() => pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    f.source.install()
    expect(f.install).toHaveBeenCalledOnce()
    await act(async () => { pending.resolve({ ...idle, installed: true, active: true, phase: 'ready' }) })
    f.restart.mockRejectedValueOnce(new Error('no shell'))
    await f.emit({ id: 'office', installed: true, active: false, phase: 'ready' })
    fireEvent.click(screen.getByRole('button', { name: '重启以启用' }))
    await act(async () => {})
    expect(screen.getByRole('alert').textContent).toBe('安装失败，请重试')
  } finally {
    pending.resolve(idle)
    f.view.unmount()
    f.status.resolve(idle)
  }
})

it('ignores an install rejection that arrives after disposal', async () => {
  const f = fixture()
  const pending = Promise.withResolvers<DesktopOfficePresentation>()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockImplementationOnce(() => pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    const snapshot = f.source.store.getSnapshot()
    f.view.unmount()
    await act(async () => { pending.reject(new Error('late')) })
    expect(f.source.store.getSnapshot()).toBe(snapshot)
  } finally {
    pending.resolve(idle)
    f.status.resolve(idle)
  }
})

it('ignores an install that settles after disposal', async () => {
  const f = fixture()
  const pending = Promise.withResolvers<DesktopOfficePresentation>()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockImplementationOnce(() => pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    const snapshot = f.source.store.getSnapshot()
    f.view.unmount()
    await act(async () => { pending.resolve({ ...idle, installed: true, active: true, phase: 'ready' }) })
    expect(f.source.store.getSnapshot()).toBe(snapshot)
  } finally {
    pending.resolve(idle)
    f.status.resolve(idle)
  }
})

it('ignores a restart rejection that arrives after disposal', async () => {
  const f = fixture()
  const pending = Promise.withResolvers<undefined>()
  try {
    await f.emit({ id: 'office', installed: true, active: false, phase: 'ready' })
    f.restart.mockImplementationOnce(() => pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '重启以启用' }))
    const snapshot = f.source.store.getSnapshot()
    f.view.unmount()
    await act(async () => { pending.reject(new Error('late')) })
    expect(f.source.store.getSnapshot()).toBe(snapshot)
  } finally {
    pending.resolve(undefined)
    f.status.resolve(idle)
  }
})

it('ignores an initial status that settles after disposal', async () => {
  const f = fixture()
  f.view.unmount()
  // The carrier's first answer arrived after the row detached.
  await act(async () => { f.status.reject(new Error('late')) })
  expect(f.source.store.getSnapshot()).toEqual({ failed: false, busy: false })
  const late = fixture()
  late.view.unmount()
  await act(async () => { late.status.resolve(idle) })
  expect(late.source.store.getSnapshot()).toEqual({ failed: false, busy: false })
})

it('reports an install rejection while mounted', async () => {
  const f = fixture()
  try {
    await act(async () => { f.status.resolve(idle) })
    f.install.mockRejectedValueOnce(new Error('Host unreachable'))
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await act(async () => {})
    expect(screen.getByRole('alert').textContent).toBe('安装失败，请重试')
    expect(screen.getByRole('button', { name: '重试' })).toBeTruthy()
  } finally { f.view.unmount(); f.status.resolve(idle) }
})
