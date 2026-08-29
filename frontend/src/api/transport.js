import { createRuntimeProfile } from '@/platform/runtime'
import { tauriTransport } from './tauriTransport'

function unavailable(operation) {
  return () => Promise.reject(new Error(`${operation} is unavailable in this runtime`))
}

export function createTransport(operations = {}) {
  return {
    invoke: operations.invoke || unavailable('invoke'),
    listen: operations.listen || unavailable('listen'),
    open: operations.open || unavailable('open'),
    openUrl: operations.openUrl || unavailable('openUrl'),
    show: operations.show || unavailable('show'),
    getRuntimeProfile: operations.getRuntimeProfile || (() => createRuntimeProfile()),
  }
}

let activeTransport = tauriTransport

export function getTransport() {
  return activeTransport
}

export function getRuntimeProfile() {
  return activeTransport.getRuntimeProfile?.() || createRuntimeProfile()
}

export function setTransport(transport) {
  activeTransport = createTransport(transport)
}

export function resetTransport() {
  activeTransport = tauriTransport
}
