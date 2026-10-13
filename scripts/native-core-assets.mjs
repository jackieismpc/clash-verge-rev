import { createHash } from 'node:crypto'
import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gunzipSync } from 'node:zlib'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const lock = JSON.parse(await fs.readFile(path.join(root, 'src-tauri/core-artifacts.json'), 'utf8'))
if (process.platform !== 'darwin' || process.arch !== 'arm64' || lock.target !== 'aarch64-apple-darwin') {
  throw new Error('Native core assets require Apple Silicon macOS')
}
const record = lock.cores.stable
const directory = path.join(root, '.work/engines')
const archive = path.join(directory, `${record.sha256}.gz`)
await fs.mkdir(directory, { recursive: true })
let bytes
try { bytes = await fs.readFile(archive) } catch (error) { if (error.code !== 'ENOENT') throw error }
const hash = (data) => createHash('sha256').update(data).digest('hex')
if (!bytes || hash(bytes) !== record.sha256) {
  if (process.argv.includes('--offline')) throw new Error('Verified pinned core archive is unavailable offline')
  const response = await fetch(record.url, { signal: AbortSignal.timeout(120_000) })
  if (!response.ok) throw new Error(`Pinned core download failed: HTTP ${response.status}`)
  bytes = Buffer.from(await response.arrayBuffer())
  if (hash(bytes) !== record.sha256) throw new Error('Pinned core archive SHA256 mismatch')
  await fs.writeFile(`${archive}.partial`, bytes)
  await fs.rename(`${archive}.partial`, archive)
}
const binary = gunzipSync(bytes)
await fs.writeFile(path.join(directory, 'mihomo.partial'), binary, { mode: 0o755 })
await fs.chmod(path.join(directory, 'mihomo.partial'), 0o755)
await fs.rename(path.join(directory, 'mihomo.partial'), path.join(directory, 'mihomo'))
await fs.writeFile(path.join(directory, 'verified.json'), JSON.stringify({
  version: record.version, archiveSHA256: record.sha256, binarySHA256: hash(binary),
}, null, 2))
console.log(`Prepared verified ${record.version} at ${path.join(directory, 'mihomo')}`)
