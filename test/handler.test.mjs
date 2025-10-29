import { ok, strictEqual, deepStrictEqual } from 'node:assert/strict'
import { test } from 'node:test'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

import { Python, Request } from '../index.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const fixturesDir = join(__dirname, 'fixtures')

test('Python', async t => {
  await t.test('constructor', () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'main:app'
    })

    ok(python instanceof Python, 'Python should be defined')
    strictEqual(python.docroot, fixturesDir, 'should set docroot correctly')
  })

  await t.test('handleRequestSync - basic ASGI app', () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'main:app'
    })

    const request = new Request({
      method: 'GET',
      url: '/test',
      headers: {
        'Content-Type': 'application/json',
        'X-Custom-Header': 'CustomValue'
      }
    })

    const response = python.handleRequestSync(request)

    strictEqual(response.status, 200, 'should return 200 status')
    strictEqual(response.headers.get('content-type'), 'text/plain', 'should have correct content-type')
    strictEqual(response.body.toString(), 'Hello, world!', 'should return correct body')
  })

  await t.test('handleRequest - async ASGI app', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'main:app'
    })

    const request = new Request({
      method: 'POST',
      url: '/test',
      headers: {
        'Content-Type': 'text/plain'
      },
      body: Buffer.from('Test request body')
    })

    const response = await python.handleRequest(request)

    strictEqual(response.status, 200, 'should return 200 status')
    strictEqual(response.headers.get('content-type'), 'text/plain', 'should have correct content-type')
  })

  await t.test('handleRequest - echo ASGI app', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'echo_app:app'
    })

    const request = new Request({
      method: 'POST',
      url: '/api/test?foo=bar',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer test-token'
      },
      body: Buffer.from(JSON.stringify({ message: 'Hello ASGI' }))
    })

    const response = await python.handleRequest(request)

    strictEqual(response.status, 200, 'should return 200 status')
    strictEqual(response.headers.get('content-type'), 'application/json', 'should have JSON content-type')
    strictEqual(response.headers.get('x-echo-method'), 'POST', 'should echo method')
    strictEqual(response.headers.get('x-echo-path'), '/api/test', 'should echo path')

    const responseBody = JSON.parse(response.body.toString())
    strictEqual(responseBody.method, 'POST', 'response should contain method')
    strictEqual(responseBody.path, '/api/test', 'response should contain path')
    deepStrictEqual(responseBody.body, JSON.stringify({ message: 'Hello ASGI' }), 'response should contain request body')
    ok(responseBody.headers['content-type'].includes('application/json'), 'should have content-type header')
    strictEqual(responseBody.headers.authorization, 'Bearer test-token', 'should have authorization header')
  })

  await t.test('handleRequest - HTTP status codes', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'status_app:app'
    })

    // Test various status codes
    for (const status of [200, 201, 400, 404, 500]) {
      const request = new Request({
        method: 'GET',
        url: `/status/${status}`
      })

      const response = await python.handleRequest(request)
      strictEqual(response.status, status, `should return ${status} status`)
      strictEqual(response.body.toString(), `Status: ${status}`, 'should return status in body')
    }
  })

  await t.test('handleRequest - streaming response', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'stream_app:app'
    })

    const request = new Request({
      method: 'GET',
      url: '/'
    })

    const response = await python.handleRequest(request)
    strictEqual(response.status, 200, 'should return 200 status')
    strictEqual(response.body.toString(), 'Chunk 1\nChunk 2\nChunk 3\nChunk 4\nChunk 5\n', 'should concatenate all chunks')
  })

  await t.test('handleRequest - root_path', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'root_path_app:app'
    })

    const request = new Request({
      method: 'GET',
      url: '/test'
    })

    const response = await python.handleRequest(request)
    strictEqual(response.status, 200, 'should return 200 status')
    strictEqual(response.headers.get('x-root-path'), fixturesDir, 'should include root_path in header')
    strictEqual(response.body.toString(), `Root path: ${fixturesDir}`, 'should include root_path in body')
  })

  await t.test('handleRequest - error handling', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'error_app:app'
    })

    // Test normal request
    const request1 = new Request({
      method: 'GET',
      url: '/ok'
    })
    const response1 = await python.handleRequest(request1)
    strictEqual(response1.status, 200, 'should handle normal request')
    strictEqual(response1.body.toString(), 'OK', 'should return OK')

    // Test error request - should throw because the ASGI app raises an exception
    const request2 = new Request({
      method: 'GET',
      url: '/error'
    })

    try {
      await python.handleRequest(request2)
      // Should not reach here
      ok(false, 'should have thrown an error')
    } catch (error) {
      // Error is expected for unhandled exceptions
      ok(error.message.includes('No response sent') || error.message.includes('Test error'), 'should fail on error')
    }
  })
})
