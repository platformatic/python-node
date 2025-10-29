import { test } from 'node:test'
import { strictEqual } from 'node:assert'
import { join } from 'node:path'

import { Python, Request } from '../index.js'

const fixturesDir = join(import.meta.dirname, 'fixtures')

test('Python - WebSocket', async (t) => {
  await t.test('basic WebSocket echo', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      websocket: true
    })

    const res = await python.handleStream(req)

    // WebSocket accepts don't have traditional HTTP status codes
    // The response iterator handles the WebSocket messages

    // Send a text message
    await req.write('Hello WebSocket!')

    // Read the echo response
    const chunk = await res.next()
    strictEqual(chunk.toString('utf8'), 'Hello WebSocket!')

    // Send another message
    await req.write('Second message')

    const chunk2 = await res.next()
    strictEqual(chunk2.toString('utf8'), 'Second message')

    // Close the connection
    await req.end()
  })

  await t.test('WebSocket binary messages', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      websocket: true
    })

    const res = await python.handleStream(req)

    // Send binary data
    const binaryData = Buffer.from([0x01, 0x02, 0x03, 0x04, 0x05])
    await req.write(binaryData)

    // Read the echo response
    const chunk = await res.next()
    strictEqual(Buffer.compare(chunk, binaryData), 0, 'Binary data should match')

    await req.end()
  })

  await t.test('WebSocket uppercase transformation', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/uppercase',
      websocket: true
    })

    const res = await python.handleStream(req)

    // Send lowercase text
    await req.write('hello world')

    // Should receive uppercase
    const chunk = await res.next()
    strictEqual(chunk.toString('utf8'), 'HELLO WORLD')

    await req.end()
  })

  await t.test('WebSocket ping-pong', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/ping-pong',
      websocket: true
    })

    const res = await python.handleStream(req)

    // Send ping
    await req.write('ping')

    // Should receive pong
    const chunk = await res.next()
    strictEqual(chunk.toString('utf8'), 'pong')

    // Send other message
    await req.write('hello')

    // Should be echoed
    const chunk2 = await res.next()
    strictEqual(chunk2.toString('utf8'), 'hello')

    await req.end()
  })

  await t.test('WebSocket immediate close', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/close',
      websocket: true
    })

    const res = await python.handleStream(req)

    // Server immediately closes after accepting
    const chunk = await res.next()

    // Should receive null/undefined indicating connection closed
    strictEqual(chunk, null, 'Connection should be closed')
  })

  await t.test('WebSocket multiple messages in sequence', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      websocket: true
    })

    const res = await python.handleStream(req)

    const messages = ['msg1', 'msg2', 'msg3', 'msg4', 'msg5']

    for (const msg of messages) {
      await req.write(msg)
      const chunk = await res.next()
      strictEqual(chunk.toString('utf8'), msg, `Should echo ${msg}`)
    }

    await req.end()
  })

  await t.test('WebSocket with headers', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      headers: {
        'Sec-WebSocket-Protocol': 'chat',
        'Sec-WebSocket-Version': '13',
        'Origin': 'http://example.com'
      },
      websocket: true
    })

    const res = await python.handleStream(req)

    // Send and receive a message to verify connection works
    await req.write('test')
    const chunk = await res.next()
    strictEqual(chunk.toString('utf8'), 'test')

    await req.end()
  })

  await t.test('Non-WebSocket request to WebSocket app', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      websocket: false  // Regular HTTP request
    })

    const res = await python.handleRequest(req)

    // Should receive 426 Upgrade Required
    strictEqual(res.status, 426)
    strictEqual(res.body.toString('utf8'), 'Upgrade Required')
  })
})
