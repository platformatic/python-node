import { test } from 'node:test'
import assert, { strictEqual, deepStrictEqual } from 'node:assert'
import { join } from 'node:path'

import { Python, Request } from '../index.js'

const fixturesDir = join(import.meta.dirname, 'fixtures')

test('Python - streaming', async (t) => {
  await t.test('handleStream - basic response', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'main:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/'
    })

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    // Collect streaming body
    let body = ''
    for await (const chunk of res) {
      body += chunk.toString('utf8')
    }
    strictEqual(body, 'Hello, world!')
  })

  await t.test('handleStream - chunked output', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'stream_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/?count=3&newlines=false'
    })

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    // Collect all chunks
    const chunks = []
    for await (const chunk of res) {
      chunks.push(chunk.toString('utf8'))
    }

    // Should have received all chunks
    strictEqual(chunks.length, 3)
    const body = chunks.join('')
    strictEqual(body, 'Chunk 1Chunk 2Chunk 3')
  })

  await t.test('handleStream - headers available immediately', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'echo_app:app'
    })

    const req = new Request({
      method: 'POST',
      url: 'http://example.com/test',
      headers: {
        'Content-Type': 'application/json'
      },
      body: Buffer.from(JSON.stringify({ status: 'ok' }))
    })

    const res = await python.handleStream(req)

    // Headers should be available immediately
    strictEqual(res.status, 200)
    strictEqual(res.headers.get('content-type'), 'application/json')

    // Body can be consumed after
    let body = ''
    for await (const chunk of res) {
      body += chunk.toString('utf8')
    }

    const responseBody = JSON.parse(body)
    strictEqual(responseBody.method, 'POST')
    strictEqual(responseBody.path, '/test')
  })

  await t.test('handleStream - POST with buffered body', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'echo_app:app'
    })

    const req = new Request({
      method: 'POST',
      url: 'http://example.com/echo',
      headers: {
        'Content-Type': 'text/plain'
      },
      body: Buffer.from('Hello from client!')
    })

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    let body = ''
    for await (const chunk of res) {
      body += chunk.toString('utf8')
    }

    const responseBody = JSON.parse(body)
    strictEqual(responseBody.body, 'Hello from client!')
  })

  await t.test('handleStream - POST with streamed body', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'echo_app:app'
    })

    const req = new Request({
      method: 'POST',
      url: 'http://example.com/echo',
      headers: {
        'Content-Type': 'text/plain'
      }
    })

    // Stream the body in chunks using write() and end()
    await req.write('Hello ')
    await req.write('from ')
    await req.write('streaming!')
    await req.end()

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    let body = ''
    for await (const chunk of res) {
      body += chunk.toString('utf8')
    }

    const responseBody = JSON.parse(body)
    strictEqual(responseBody.body, 'Hello from streaming!')
  })

  await t.test('handleStream - empty response', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'stream_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/empty'
    })

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    let body = ''
    for await (const chunk of res) {
      body += chunk.toString('utf8')
    }
    strictEqual(body, '')
  })

  await t.test('handleStream - large streaming response', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'stream_app:app'
    })

    const chunkCount = 100
    const req = new Request({
      method: 'GET',
      url: `http://example.com/?count=${chunkCount}`
    })

    const res = await python.handleStream(req)
    strictEqual(res.status, 200)

    // Collect all chunks
    const chunks = []
    for await (const chunk of res) {
      chunks.push(chunk.toString('utf8'))
    }

    // Generate expected chunks - each is "Chunk N\n"
    const expectedChunks = Array.from({ length: chunkCount }, (_, i) => `Chunk ${i + 1}\n`)

    // Verify we received all expected chunks
    deepStrictEqual(chunks, expectedChunks)
  })

  await t.test('handleStream - error handling', async (t) => {
    await t.test('exception before response.start', async () => {
      const python = new Python({
        docroot: fixturesDir,
        appTarget: 'error_app:app'
      })

      const req = new Request({
        method: 'GET',
        url: 'http://example.com/error'
      })

      // Should throw error before response.start is sent
      await assert.rejects(
        async () => await python.handleStream(req),
        (err) => {
          // Verify error message contains "Test error"
          return err.message.includes('Test error')
        },
        'Should throw Python exception before response.start'
      )
    })

    await t.test('exception after response.start during streaming', async () => {
      const python = new Python({
        docroot: fixturesDir,
        appTarget: 'stream_error_app:app'
      })

      const req = new Request({
        method: 'GET',
        url: 'http://example.com/error-during-stream'
      })

      const res = await python.handleStream(req)
      strictEqual(res.status, 200, 'should return 200 status (response.start sent)')

      // Collect chunks until error
      const chunks = []
      await assert.rejects(
        async () => {
          for await (const chunk of res) {
            chunks.push(chunk.toString('utf8'))
          }
        },
        (err) => {
          // Verify error message contains "Error during streaming"
          // err might be a string or Error object
          const errorMsg = typeof err === 'string' ? err : err.message
          return errorMsg.includes('Error during streaming')
        },
        'Should propagate exception as error in stream'
      )

      // Verify we received some chunks before the error
      strictEqual(chunks.length, 2, 'should receive 2 chunks before error')
      strictEqual(chunks[0], 'Chunk 1\n')
      strictEqual(chunks[1], 'Chunk 2\n')
    })
  })
})
