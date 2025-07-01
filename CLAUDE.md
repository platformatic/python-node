# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust-based Node.js native addon that enables running **ASGI-compatible Python applications** within Node.js environments. It uses NAPI-RS to create Node.js bindings for Rust code that integrates with Python via PyO3.

## Build Commands

```bash
# Install dependencies
npm install

# Build release version for current platform
npm run build

# Build debug version
npm run build:debug

# Run tests
npm test

# Lint code
npm run lint
```

## Architecture

The project implements a complete ASGI 3.0 server that bridges Node.js, Rust, and Python:

**Core Components:**
- **Rust Backend** (`/src/lib.rs`): Main NAPI addon exposing `PythonHandler` class to JavaScript
- **ASGI Implementation** (`/src/asgi/mod.rs`): Complete ASGI protocol handler with HTTP, WebSocket, and Lifespan support
- **Protocol Modules** (`/src/asgi/`): Individual ASGI message types and connection scopes
- **Node.js Interface** (`/index.js`, `/index.d.ts`): Auto-generated NAPI bindings

**Key Architecture Details:**
- Uses PyO3 with `pyo3-async-runtimes` for Python async integration
- Supports Python virtual environments via `VIRTUAL_ENV` environment variable
- Implements tokio runtime management for async Python code execution
- Cross-platform builds for macOS (arm64, x64) and Linux (x64-gnu)

## Python Integration

**ASGI Support:**
- Full ASGI 3.0 specification implementation
- HTTP request/response handling with streaming support
- WebSocket connections (implementation in progress)
- Lifespan management for application startup/shutdown

**Python Environment:**
- Automatically detects and uses Python virtual environments
- Dynamically discovers Python site-packages directories
- Supports uvloop for improved async performance when available
- Handles Python symbol loading on Linux systems

## Testing

**Test Framework:** Node.js built-in test runner
**Test Files:**
- `/test/handler.test.mjs`: Core ASGI functionality, HTTP methods, headers, error handling
- `/test/concurrency.test.mjs`: Concurrent request processing and performance testing
- `/test/fixtures/`: Python ASGI test applications for various scenarios

**Test Applications:**
- `main.py`: Basic "Hello, world!" ASGI app
- `echo_app.py`: Request echo service with headers and body reflection
- `status_app.py`: HTTP status code testing
- `stream_app.py`: Chunked response streaming
- `error_app.py`: Exception handling testing

## Important Implementation Details

**Async Runtime Management:**
- Uses `pyo3_async_runtimes::tokio` to bridge Rust tokio and Python asyncio
- Event loop setup with uvloop preference for performance
- Handles Python coroutine execution within tokio runtime context

**Request/Response Flow:**
1. Node.js HTTP request → Rust `Request` struct
2. ASGI scope creation with HTTP connection details
3. Python ASGI app invocation with scope, receive, send callables
4. Async message passing between Rust and Python
5. Response assembly from ASGI send messages

**Error Handling:**
- Comprehensive `HandlerError` enum covering IO, Python, and HTTP errors
- Proper error propagation from Python exceptions to Node.js
- Graceful handling of malformed ASGI responses

## Dependencies

**Key Rust Dependencies:**
- `pyo3` (0.25.1): Python integration with experimental async support
- `pyo3-async-runtimes` (0.25.0): Tokio-Python async runtime bridge
- `tokio` (1.45.1): Async runtime with full feature set
- `napi` (3.0.0-beta.8): Node.js addon interface

**Git Dependencies:**
- `http-handler`: HTTP request/response primitives
- `http-rewriter`: HTTP request transformation utilities

## Current Status

This is a **production-ready** implementation with:
- ✅ Complete ASGI 3.0 protocol implementation
- ✅ Full HTTP request/response handling
- ✅ Async Python code execution
- ✅ Virtual environment support
- ✅ Comprehensive test coverage
- ✅ Cross-platform builds
- ✅ Performance optimizations with uvloop

**Build System:** Uses PNPM for Node.js dependencies and Cargo for Rust compilation.
**License:** MIT (package.json) / Apache-2.0 (Cargo.toml) - dual licensed