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

// Function to find the correct .node file for this platform
function findNodeFile() {
  // Only run on Linux - other platforms don't need soname fixing
  if (platform !== 'linux') return

  // Map Node.js arch to napi-rs target
  const archMap = {
    'x64': 'x86_64-unknown-linux-gnu',
    'arm64': 'aarch64-unknown-linux-gnu',
  }

  const target = archMap[arch]
  if (!target) return

  // Try to find the .node file with various naming patterns
  const possiblePaths = [
    // Specific platform builds
    path.join(__dirname, `python-node.${target}.node`),
    path.join(__dirname, `index.${target}.node`),
    path.join(__dirname, `npm/${target}/python-node.${target}.node`),
    // Generic .node files (common during testing)
    path.join(__dirname, 'python-node.node'),
    path.join(__dirname, 'index.node'),
    // Look for any .node file in current directory
    ...fs.readdirSync(__dirname)
      .filter(f => f.endsWith('.node') && !f.includes('node_modules'))
      .map(f => path.join(__dirname, f))
  ]

  for (const nodePath of possiblePaths) {
    if (fs.existsSync(nodePath)) {
      return nodePath
    }
  }

  // Return undefined if no .node file found - this is expected during development
  return undefined
}

// Get the node file path
const nodeFilePath = findNodeFile()
if (!nodeFilePath) {
  if (isDevInstall()) {
    // No .node file found during dev install - this is expected, skip silently
    console.log('No .node file found during development install, skipping soname fix')
    process.exit(0)
  } else {
    // No .node file found when installed as dependency - this is an error
    console.error('Error: Could not find *.node file to fix soname')
    process.exit(1)
  }
}

// Check if WASM file exists
const wasmPath = path.join(__dirname, 'fix-python-soname.wasm')
if (!fs.existsSync(wasmPath)) {
  if (isDevInstall()) {
    // WASM file not found during dev install - this is expected, skip with warning
    console.log('WASM file not found during development install, skipping soname fix')
    process.exit(0)
  } else {
    // WASM file not found when installed as dependency - this is an error
    console.error('Error: fix-python-soname.wasm not found')
    process.exit(1)
  }
}

console.log(`Running soname fix on ${nodeFilePath}`)

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
