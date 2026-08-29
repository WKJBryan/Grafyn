const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..', '..')
const errors = []

function read(relativePath) {
  const absolutePath = path.join(root, relativePath)
  try {
    return fs.readFileSync(absolutePath, 'utf8')
  } catch (error) {
    errors.push(`${relativePath}: ${error.code === 'ENOENT' ? 'missing' : error.message}`)
    return ''
  }
}

function check(condition, message) {
  if (!condition) errors.push(message)
}

function normalized(text) {
  return `${text.replace(/\r\n/g, '\n').replace(/\n+$/, '')}\n`
}

function sha256(text) {
  return crypto.createHash('sha256').update(normalized(text), 'utf8').digest('hex')
}

function parseJson(relativePath) {
  const text = read(relativePath)
  if (!text) return {}
  try {
    return JSON.parse(text)
  } catch (error) {
    errors.push(`${relativePath}: invalid JSON (${error.message})`)
    return {}
  }
}

const mplSha256 = '3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04'
const apacheSha256 = 'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30'
const dcoSha256 = 'f7ac75b443f4ca16b503241344b41aeff9503b0c30bedc2b119551d83cb0fa90'

const license = read('LICENSE')
const mplLicense = read('LICENSES/MPL-2.0.txt')
const apacheLicense = read('LICENSES/Apache-2.0.txt')
const contributing = read('CONTRIBUTING.md')
const trademarks = read('TRADEMARKS.md')
const relicensing = read('RELICENSING.md')
const readme = read('README.md')
const cargoToml = read('frontend/src-tauri/Cargo.toml')
const cargoLock = read('frontend/src-tauri/Cargo.lock')
const frontendPackage = parseJson('frontend/package.json')
const frontendLock = parseJson('frontend/package-lock.json')
const e2ePackage = parseJson('e2e/package.json')
const e2eLock = parseJson('e2e/package-lock.json')
const dcoBlock = normalized(contributing).match(
  /```text\n(Developer Certificate of Origin\nVersion 1\.1[\s\S]*?)\n```/,
)?.[1]

check(sha256(license) === mplSha256, 'LICENSE must be the unmodified official MPL-2.0 text')
check(sha256(mplLicense) === mplSha256, 'LICENSES/MPL-2.0.txt must be the unmodified official MPL-2.0 text')
check(normalized(license) === normalized(mplLicense), 'LICENSE and LICENSES/MPL-2.0.txt must match')
check(sha256(apacheLicense) === apacheSha256, 'LICENSES/Apache-2.0.txt must be the unmodified official Apache-2.0 text')

check(frontendPackage.license === 'MPL-2.0', 'frontend/package.json must declare MPL-2.0')
check(frontendLock.packages?.['']?.license === 'MPL-2.0', 'frontend/package-lock.json root package must declare MPL-2.0')
check(e2ePackage.license === 'MPL-2.0', 'e2e/package.json must declare MPL-2.0')
check(e2eLock.packages?.['']?.license === 'MPL-2.0', 'e2e/package-lock.json root package must declare MPL-2.0')
check(/^license = "MPL-2\.0"$/m.test(cargoToml), 'frontend/src-tauri/Cargo.toml must declare MPL-2.0')
check(/\[\[package\]\]\r?\nname = "grafyn"\r?\nversion = "0\.3\.0"/m.test(cargoLock), 'frontend/src-tauri/Cargo.lock must contain the locked Grafyn package')

check(
  dcoBlock && sha256(dcoBlock) === dcoSha256,
  'CONTRIBUTING.md must contain the complete unmodified official DCO 1.1 text',
)
check(contributing.includes('inbound equals outbound'), 'CONTRIBUTING.md must state inbound equals outbound')
check(contributing.includes('Signed-off-by:'), 'CONTRIBUTING.md must explain DCO sign-off')
check(!/contributor license agreement|\bCLA\b/i.test(contributing), 'CONTRIBUTING.md must not introduce a CLA')

check(/Grafyn name and logo/i.test(trademarks), 'TRADEMARKS.md must reserve the Grafyn name and logo')
check(/truthful(?:ly)?[^.]*compatib|compatib[^.]*truthful/i.test(trademarks), 'TRADEMARKS.md must allow truthful compatibility descriptions')
check(/fork/i.test(trademarks), 'TRADEMARKS.md must address forks')

check(relicensing.includes('2026-08-29'), 'RELICENSING.md must record the authorization date')
check(/GNU Affero General Public License[^\n]*3|AGPL-3\.0/i.test(relicensing), 'RELICENSING.md must identify the previous root public license')
check(/earlier recipients[^\n]*retain|prior grants[^\n]*remain/i.test(relicensing), 'RELICENSING.md must preserve earlier recipients\' prior grants')
check(/Git identity is evidence, not legal proof/i.test(relicensing), 'RELICENSING.md must state the provenance evidence limit')
check(/non-merge commits/i.test(relicensing), 'RELICENSING.md must record the non-merge commit audit')
check(/vendored|copied source/i.test(relicensing), 'RELICENSING.md must record the vendored/copied-source notice audit')

check(/license-MPL--2\.0/i.test(readme), 'README.md license badge must say MPL-2.0')
check(/open client and local core/i.test(readme), 'README.md must describe the open client and local core')
check(/future[^\n]*hosted service/i.test(readme), 'README.md must describe the hosted service as future work')
check(/not (?:currently )?available|not (?:yet )?available/i.test(readme), 'README.md must not imply that the hosted service is available')
check(!/GPL-3\.0|AGPL-3\.0/i.test(readme), 'README.md must not advertise a legacy license')

if (errors.length) {
  console.error('License alignment check failed:')
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}

console.log('License alignment check passed.')
