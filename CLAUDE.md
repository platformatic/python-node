# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust-based Node.js native addon project that aims to integrate Python capabilities into Node.js applications. It uses NAPI-RS to create Node.js bindings for Rust code.

## Build Commands

```bash
# Install dependencies
yarn install

# Build release version for current platform
yarn build

# Build debug version
yarn build:debug

# Run tests
yarn test
```

## Architecture

The project structure follows NAPI-RS conventions:
- **Rust code**: `/src/lib.rs` contains the native addon implementation
- **Node.js interface**: `/index.js` and `/index.d.ts` provide the JavaScript API
- **Cross-platform builds**: Configured for macOS (arm64, x64) and Linux (x64-gnu)

Key components:
- `PythonHandler` struct in Rust handles Python integration (currently incomplete)
- Uses local dependencies: `http-handler` and `http-rewriter` from parent directory
- Built with Rust edition 2024

## Current Implementation Status

The project is in early development:
- The `PythonHandler` currently acts as an echo server (see TODO in `/src/lib.rs`)
- Test file references a non-existent `sum` function
- Python integration functionality is not yet implemented

## Important Notes

1. **License inconsistency**: package.json specifies MIT, while Cargo.toml specifies Apache-2.0
2. **Package manager**: Uses Yarn 4.9.2 (not npm)
3. **Local dependencies**: Depends on packages in parent directory (`../http-handler`, `../http-rewriter`)
4. **Test framework**: AVA with 3-minute timeout per test