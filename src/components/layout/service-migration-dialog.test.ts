import { useState } from 'react'
import { beforeEach, expect, it, vi } from 'vitest'

import { getRuntimeState, type RunState } from '@/services/cmds'

import { ServiceMigrationDialog } from './service-migration-dialog'

const store = vi.hoisted(() => ({
  dispatch: vi.fn(),
  runState: null as RunState | null,
}))

vi.mock('react', () => ({ useState: vi.fn() }))
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))
vi.mock('@mui/material', () => ({ Alert: 'div' }))
vi.mock('@/components/base', () => ({ BaseDialog: 'dialog' }))
vi.mock('@/store/app-store-context', () => ({
  useAppReads: () => ({ readRunState: getRuntimeState }),
  useRunState: () => store.runState,
}))
vi.mock('@/services/cmds', () => ({
  getRuntimeState: vi.fn(),
  installService: vi.fn(),
  restartCore: vi.fn(),
}))
vi.mock('@/services/notice-service', () => ({
  showNotice: { warning: vi.fn(), error: vi.fn(), success: vi.fn() },
}))

beforeEach(() => {
  vi.clearAllMocks()
  store.runState = null
  vi.mocked(useState)
    .mockReturnValueOnce([false, vi.fn()])
    .mockReturnValueOnce([false, vi.fn()])
    .mockReturnValueOnce([true, vi.fn()])
})

it.each([
  { service: 'ready', mode: 'NotRunning' },
  { service: 'unknown', mode: 'NotRunning' },
  { service: 'unavailable', mode: 'Service' },
  { service: 'unavailable', mode: 'NotRunning', opInFlight: true },
  { service: 'unavailable', mode: 'NotRunning', pendingAction: 'reinstall' },
] satisfies Partial<RunState>[])(
  'does not offer Sidecar continuation from %j',
  (state) => {
    store.runState = state as unknown as RunState
    const dialog = ServiceMigrationDialog()
    expect(dialog.props.open).toBe(true)
    expect(dialog.props.disableCancel).toBe(true)
  },
)

it.each([
  { service: 'notInstalled', mode: 'NotRunning' },
  { service: 'versionMismatch', mode: 'NotRunning' },
  { service: 'unavailable', mode: 'NotRunning' },
  { service: 'notInstalled', mode: 'Sidecar', pendingAction: 'install' },
] satisfies Partial<RunState>[])(
  'offers Sidecar continuation from %j',
  (state) => {
    store.runState = state as unknown as RunState
    expect(ServiceMigrationDialog().props.disableCancel).toBe(false)
  },
)
