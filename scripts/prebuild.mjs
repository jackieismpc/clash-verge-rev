import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs/promises'
import path from 'node:path'
import { gunzipSync } from 'node:zlib'

import { extract } from 'tar'

import { resolveServiceRelease } from './service-release.mjs'
import { log_info, log_success } from './utils.mjs'

const ROOT = process.cwd()
const TARGET = 'aarch64-apple-darwin'
const args = process.argv.slice(2)
const FORCE = args.includes('--force') || args.includes('-f')
const targetArguments = args.filter((arg) => arg !== '--force' && arg !== '-f')
if (
  process.platform !== 'darwin' ||
  process.arch !== 'arm64' ||
  targetArguments.length > 1 ||
  (targetArguments.length === 1 && targetArguments[0] !== TARGET)
) {
  throw new Error('prebuild supports aarch64-apple-darwin on Apple Silicon macOS only')
}
const majorVersion = Number(
  execFileSync('sw_vers', ['-productVersion'], { encoding: 'utf8' }).split('.')[0],
)
if (!Number.isInteger(majorVersion) || majorVersion < 26) {
  throw new Error('prebuild requires macOS 26 or later')
}
const TEMP_DIR = path.join(ROOT, '.work', 'prebuild')
const RESOURCES_DIR = path.join(ROOT, 'src-tauri', 'resources')
const SIDECAR_DIR = path.join(ROOT, 'src-tauri', 'sidecar')

async function sha256(file) {
  return createHash('sha256').update(await fs.readFile(file)).digest('hex')
}

async function download(url, destination, expectedHash) {
  if (!FORCE) {
    try {
      await fs.access(destination)
      if (!expectedHash || (await sha256(destination)) === expectedHash) return
    } catch (error) {
      if (error.code !== 'ENOENT') throw error
    }
  }
  const response = await fetch(url)
  if (!response.ok) throw new Error(`Download failed: ${url}: ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  if (expectedHash && createHash('sha256').update(bytes).digest('hex') !== expectedHash) {
    throw new Error(`SHA256 mismatch: ${url}`)
  }
  await fs.mkdir(path.dirname(destination), { recursive: true })
  const temporary = `${destination}.partial`
  try {
    await fs.writeFile(temporary, bytes)
    await fs.rename(temporary, destination)
  } finally {
    await fs.rm(temporary, { force: true })
  }
}

async function prepareCore(channel, record) {
  if (
    !record ||
    typeof record.version !== 'string' ||
    !record.version ||
    typeof record.url !== 'string' ||
    !record.url.startsWith('https://github.com/MetaCubeX/mihomo/releases/download/') ||
    !/^[0-9a-f]{64}$/.test(record.sha256 ?? '')
  ) {
    throw new Error(`Pinned ${channel} core requires version, release URL, and archive SHA256`)
  }
  const name = channel === 'alpha' ? 'verge-mihomo-alpha' : 'verge-mihomo'
  const archive = path.join(TEMP_DIR, `${name}-${record.sha256}.gz`)
  await download(record.url, archive, record.sha256)
  const binary = gunzipSync(await fs.readFile(archive))
  const destination = path.join(SIDECAR_DIR, `${name}-${TARGET}`)
  await fs.mkdir(SIDECAR_DIR, { recursive: true })
  const temporary = `${destination}.partial`
  try {
    await fs.writeFile(temporary, binary, { mode: 0o755 })
    await fs.chmod(temporary, 0o755)
    await fs.rename(temporary, destination)
  } finally {
    await fs.rm(temporary, { force: true })
  }
  log_success(`Prepared pinned ${channel} core ${record.version}`)
}

const SERVICE_BINARIES = [
  'clash-verge-service',
  'clash-verge-service-install',
  'clash-verge-service-uninstall',
]

async function findExtractedFile(directory, filename) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name)
    if (entry.isFile() && entry.name === filename) return entryPath
    if (entry.isDirectory()) {
      const found = await findExtractedFile(entryPath, filename)
      if (found) return found
    }
  }
  return null
}

async function prepareService() {
  const manifest = await fs.readFile(path.join(ROOT, 'src-tauri', 'Cargo.toml'), 'utf8')
  const dependency = manifest
    .split(/\r?\n/)
    .find((line) => line.trimStart().startsWith('clash_verge_service_ipc ='))
  const source = dependency?.match(/\bpath\s*=\s*"([^"]+)"/)?.[1]
  await fs.mkdir(RESOURCES_DIR, { recursive: true })
  let sourceDirectory
  if (source) {
    const targetDirectory = path.join(ROOT, '.work', 'target', 'bundled-service')
    execFileSync('cargo', [
      'build',
      '--manifest-path', path.resolve(ROOT, 'src-tauri', source, 'Cargo.toml'),
      '--target-dir', targetDirectory,
      '--target', TARGET,
      '--locked',
      '--release',
      '--features', 'standalone,client',
      '--bins',
    ], {
      stdio: 'inherit',
      env: {
        ...process.env,
        CARGO_HOME: process.env.CARGO_HOME || path.join(ROOT, '.work', 'cargo'),
        TMPDIR: process.env.TMPDIR || path.join(ROOT, '.work', 'tmp'),
        MACOSX_DEPLOYMENT_TARGET: '26.0',
      },
    })
    sourceDirectory = path.join(targetDirectory, TARGET, 'release')
  } else {
    const release = resolveServiceRelease(manifest, TARGET, 'darwin')
    const archive = path.join(TEMP_DIR, release.archiveFile)
    await download(release.downloadURL, archive)
    sourceDirectory = path.join(TEMP_DIR, `service-${release.version}`)
    await fs.rm(sourceDirectory, { force: true, recursive: true })
    await fs.mkdir(sourceDirectory, { recursive: true })
    await extract({ cwd: sourceDirectory, file: archive })
  }
  for (const name of SERVICE_BINARIES) {
    const sourceFile = await findExtractedFile(sourceDirectory, name)
    if (!sourceFile) throw new Error(`Service bundle is missing ${name}`)
    const destination = path.join(RESOURCES_DIR, name)
    await fs.copyFile(sourceFile, destination)
    await fs.chmod(destination, 0o755)
  }
  log_success('Prepared macOS service helper resources')
}

async function prepareResources() {
  const base = 'https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest'
  for (const [destination, asset] of [
    ['Country.mmdb', 'country.mmdb'],
    ['ASN.mmdb', 'GeoLite2-ASN.mmdb'],
    ['geosite.dat', 'geosite.dat'],
    ['geoip.dat', 'geoip.dat'],
  ]) {
    await download(`${base}/${asset}`, path.join(RESOURCES_DIR, destination))
  }
  for (const name of ['set_dns.sh', 'unset_dns.sh']) {
    await fs.copyFile(path.join(ROOT, 'scripts', name), path.join(RESOURCES_DIR, name))
  }
}

async function run() {
  const lock = JSON.parse(await fs.readFile(path.join(ROOT, 'src-tauri', 'core-artifacts.json'), 'utf8'))
  if (lock.target !== TARGET) throw new Error('Core artifact lock must target aarch64-apple-darwin')
  for (const directory of [TEMP_DIR, path.join(ROOT, '.work', 'cargo'), path.join(ROOT, '.work', 'tmp')]) {
    await fs.mkdir(directory, { recursive: true })
  }
  await prepareCore('stable', lock.cores?.stable)
  await prepareCore('alpha', lock.cores?.alpha)
  await prepareService()
  await prepareResources()
  log_info('macOS Apple Silicon resources are ready')
}

run().catch((error) => {
  console.error(`prebuild failed: ${error.message}`)
  process.exitCode = 1
})
