import { createRuntimeProfile } from '@/platform/runtime'
import { shallowRef } from 'vue'
import {
  createE2eTransport,
  resolveE2eRuntimePlatform,
  resolveE2eTransportConfiguration,
} from './e2eTransport'
import { getTauriRuntimeProfile, tauriTransport } from './tauriTransport'

function unavailable(operation) {
  return () => Promise.reject(new Error(`${operation} is unavailable in this runtime`))
}

export function createTransport(operations = {}) {
  return {
    invoke: operations.invoke || unavailable('invoke'),
    listen: operations.listen || unavailable('listen'),
    openExternal: operations.openExternal || unavailable('openExternal'),
    showMainWindow: operations.showMainWindow || unavailable('showMainWindow'),
  }
}

const e2eConfiguration = import.meta.env.DEV
  ? resolveE2eTransportConfiguration({
      enabled: true,
      runtimeUrl: import.meta.env.VITE_GRAFYN_E2E_RUNTIME_URL,
      token: import.meta.env.VITE_GRAFYN_E2E_RUNTIME_TOKEN,
    })
  : null
const e2eRuntimePlatform = e2eConfiguration
  ? resolveE2eRuntimePlatform(globalThis.location?.search ?? '')
  : null

function getE2eRuntimeProfile() {
  return createRuntimeProfile({
    isTauri: true,
    platform: e2eRuntimePlatform === 'android' ? 'android' : 'windows',
    nativePlugins: false,
  })
}

const defaultTransport = e2eConfiguration
  ? createTransport(createE2eTransport({
      ...e2eConfiguration,
      profileResolver: () => e2eRuntimePlatform,
    }))
  : tauriTransport
const defaultRuntimeProfile = e2eConfiguration
  ? getE2eRuntimeProfile
  : getTauriRuntimeProfile

let activeTransport = defaultTransport
let activeRuntimeProfile = defaultRuntimeProfile
const activeRuntimeStatus = shallowRef(null)

export function getTransport() {
  return activeTransport
}

export function getRuntimeProfile() {
  return activeRuntimeProfile?.() || createRuntimeProfile()
}

export function getRuntimeStatus() {
  return activeRuntimeStatus.value
}

export function setTransport(transport) {
  activeTransport = createTransport(transport)
}

export function setRuntimeProfile(profile) {
  activeRuntimeProfile = typeof profile === 'function' ? profile : () => profile
}

export function setRuntimeStatus(status) {
  activeRuntimeStatus.value = status
}

export function resetTransport() {
  activeTransport = defaultTransport
  activeRuntimeProfile = defaultRuntimeProfile
  activeRuntimeStatus.value = null
}
