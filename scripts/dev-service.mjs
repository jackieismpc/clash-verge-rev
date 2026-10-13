import { spawn } from 'node:child_process'
import { access, mkdir } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const developmentServiceDirectoryEnvironment =
  'CLASH_VERGE_DEV_SERVICE_DIR'
const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const serviceRepository = resolve(
  repositoryRoot,
  '..',
  'clash-verge-service-ipc',
)
const serviceManifest = join(serviceRepository, 'Cargo.toml')
export const developmentServiceWatchPaths = [
  'src',
  'resources',
  'Cargo.toml',
  'Cargo.lock',
].map((name) => join(serviceRepository, name))

function run(command, args, options = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      env: options.env ?? process.env,
      stdio: options.stdio ?? 'inherit',
    })
    let stdout = ''
    child.stdout?.setEncoding('utf8')
    child.stdout?.on('data', (chunk) => {
      stdout += chunk
    })
    child.once('error', reject)
    child.once('close', (code, signal) => {
      if (code === 0) {
        resolvePromise(stdout)
        return
      }
      const error = new Error(
        `${command} failed with ${signal ? `signal ${signal}` : `exit code ${code}`}`,
      )
      error.exitCode = code
      reject(error)
    })
  })
}

export async function prepareDevelopmentService() {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    throw new Error('Development service requires macOS on Apple Silicon')
  }
  await access(serviceManifest)
  const env = {
    ...process.env,
    CARGO_HOME: process.env.CARGO_HOME || join(repositoryRoot, '.work', 'cargo'),
    TMPDIR: process.env.TMPDIR || join(repositoryRoot, '.work', 'tmp'),
    MACOSX_DEPLOYMENT_TARGET: '26.0',
  }
  for (const name of ['CARGO_HOME', 'TMPDIR']) {
    await mkdir(env[name], { recursive: true })
  }
  const output = await run(
    'cargo',
    [
      'build',
      '--manifest-path',
      serviceManifest,
      '--target-dir',
      process.env.CARGO_TARGET_DIR ||
        join(repositoryRoot, '.work', 'target', 'development-service'),
      '--target',
      'aarch64-apple-darwin',
      '--locked',
      '--features',
      'standalone,client,development-channel',
      '--bins',
      '--message-format=json-render-diagnostics',
    ],
    { stdio: ['inherit', 'pipe', 'inherit'], env },
  )

  const artifacts = new Map()
  for (const line of output.split(/\r?\n/).filter(Boolean)) {
    const artifact = JSON.parse(line)
    if (
      artifact.reason === 'compiler-artifact' &&
      resolve(artifact.manifest_path) === serviceManifest &&
      artifact.target.kind.includes('bin') &&
      artifact.executable
    ) {
      artifacts.set(artifact.target.name, resolve(artifact.executable))
    }
  }
  const executables = [
    'clash-verge-service',
    'clash-verge-service-install',
    'clash-verge-service-uninstall',
    'service-integration-driver',
  ].map((name) => {
    const executable = artifacts.get(name)
    if (!executable)
      throw new Error(`Cargo did not report the ${name} executable`)
    return executable
  })
  const serviceDirectory = dirname(executables[0])
  for (const executable of executables) {
    if (dirname(executable) !== serviceDirectory) {
      throw new Error(
        'Development service tools were built in different directories',
      )
    }
    await access(executable)
  }
  return serviceDirectory
}

export async function ensureDevelopmentService({
  platform = process.platform,
} = {}) {
  if (platform !== 'darwin') throw new Error('Development service requires macOS')
  const serviceDirectory = await prepareDevelopmentService()
  const installer = join(
    serviceDirectory,
    'clash-verge-service-install',
  )
  const host = 'aarch64-apple-darwin'
  const args = ['--prepare-install', '--ensure']
  for (const name of ['verge-mihomo', 'verge-mihomo-alpha']) {
    args.push(
      '--core',
      name,
      join(
        repositoryRoot,
        'src-tauri',
        'sidecar',
        `${name}-${host}`,
      ),
    )
  }
  await run(installer, args)
  return serviceDirectory
}
