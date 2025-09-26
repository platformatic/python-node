#!/usr/bin/env node

/**
 * Post-install script to fix Python library soname on Linux
 *
 * This script automatically detects the system-installed Python version
 * and updates the .node binary to link against the correct libpython.so.
 *
 * Only runs on Linux - other platforms don't need soname fixing.
 * Uses a WASM version of the arwen ELF patcher for cross-platform compatibility.
 */

const { WASI } = require('wasi')
const fs = require('fs')
const path = require('path')
const os = require('os')

const platform = os.platform()
const arch = os.arch()

// Function to detect if this is a development install vs dependency install
function isDevInstall() {
  const env = process.env

  // Method 1: Check if INIT_CWD and PWD are the same (local dev install)
  if (env.INIT_CWD && env.PWD) {
    if (env.INIT_CWD === env.PWD || env.INIT_CWD.indexOf(env.PWD) === 0) {
      return true
    }
  }

  // Method 2: Check for .git folder existence (dev environment)
  if (fs.existsSync(path.join(__dirname, '.git'))) {
    return true
  }

  // Method 3: Check if we're in production mode
  if (env.NODE_ENV === 'production' || env.npm_config_production) {
    return false
  }

  return false
}

// Only patch on Linux and macOS
if (platform !== 'linux' && platform !== 'darwin') {
  console.log(`No need to fix soname on platform: ${platform}`)
  process.exit(0)
}

// Get the node file path based on platform
const nodeFilePath = platform === 'linux'
  ? path.join(__dirname, `python-node.linux-${arch}-gnu.node`)
  : path.join(__dirname, `python-node.darwin-${arch}.node`)
if (!fs.existsSync(nodeFilePath)) {
  if (isDevInstall()) {
    // No .node file found during dev install - this is expected, skip silently
    console.log(`${nodeFilePath} not found during development install, skipping binary patching`)
    process.exit(0)
  } else {
    // No .node file found when installed as dependency - this is an error
    console.error(`Error: Could not find "${nodeFilePath}" to patch binary`)
    process.exit(1)
  }
}

// Check if WASM file exists
const wasmPath = path.join(__dirname, 'fix-python-soname.wasm')
if (!fs.existsSync(wasmPath)) {
  if (isDevInstall()) {
    // WASM file not found during dev install - this is expected, skip with warning
    console.log('WASM file not found during development install, skipping binary patching')
    process.exit(0)
  } else {
    // WASM file not found when installed as dependency - this is an error
    console.error('Error: fix-python-soname.wasm not found')
    process.exit(1)
  }
}

console.log(`Running binary patch on ${nodeFilePath}`)

// Create a WASI instance
const wasi = new WASI({
  version: 'preview1',
  args: ['fix-python-soname', nodeFilePath],
  env: process.env,
  preopens: {
    '/': '/',
  }
})

async function runSonameFixer() {
  try {
    const wasm = fs.readFileSync(wasmPath);
    const { instance } = await WebAssembly.instantiate(wasm, {
      wasi_snapshot_preview1: wasi.wasiImport
    });

    // Run the WASI module
    process.exit(wasi.start(instance))
  } catch (error) {
    console.error('Error: Failed to run soname fixer:', error.message)
    process.exit(1) // Fail hard when installed as dependency
  }
}

runSonameFixer()
