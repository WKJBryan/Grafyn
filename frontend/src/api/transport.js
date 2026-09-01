import { createRuntimeProfile } from '@/platform/runtime'
import { shallowRef } from 'vue'
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

let activeTransport = tauriTransport
let activeRuntimeProfile = getTauriRuntimeProfile
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
  activeTransport = tauriTransport
  activeRuntimeProfile = getTauriRuntimeProfile
  activeRuntimeStatus.value = null
}
