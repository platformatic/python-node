import { test } from 'node:test'
import { strictEqual } from 'node:assert'
import { join } from 'node:path'
import { createServer, request as httpRequest } from 'node:http'
import { once } from 'node:events'

import { Python, Request } from '../index.js'

const fixturesDir = join(import.meta.dirname, 'fixtures')

test('Python - WebSocket Integration', async (t) => {
  await t.test('HTTP upgrade to WebSocket simulation', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    // Create a simple HTTP server
    const server = createServer(async (nodeReq, nodeRes) => {
      // Check if this is a WebSocket upgrade request
      const isUpgrade = nodeReq.headers.upgrade?.toLowerCase() === 'websocket'

      if (isUpgrade) {
        // In a real implementation, you'd:
        // 1. Perform WebSocket handshake
        // 2. Switch protocols
        // 3. Forward socket data to Python

        // For this test, we simulate by creating a WebSocket request
        const req = new Request({
          method: nodeReq.method,
          url: `http://${nodeReq.headers.host}${nodeReq.url}`,
          headers: nodeReq.headers,
          websocket: true
        })

        const res = await python.handleStream(req)

        // Simulate sending a message through the WebSocket
        await req.write('Integration test message')

        // Read response
        const chunk = await res.next()
        const response = chunk.toString('utf8')

        await req.end()

        // Send response back through HTTP for test purposes
        nodeRes.writeHead(200, { 'Content-Type': 'text/plain' })
        nodeRes.end(response)
      } else {
        // Regular HTTP request
        const req = new Request({
          method: nodeReq.method,
          url: `http://${nodeReq.headers.host}${nodeReq.url}`,
          headers: nodeReq.headers,
          websocket: false
        })

        const res = await python.handleRequest(req)
        nodeRes.writeHead(res.status, Object.fromEntries(res.headers.entries()))
        nodeRes.end(res.body)
      }
    })

    server.listen(0)
    await once(server, 'listening')

    const { port } = server.address()

    try {
      // Test WebSocket upgrade request using http.request (fetch doesn't support upgrade headers)
      const upgradeResponse = await new Promise((resolve, reject) => {
        const req = httpRequest({
          hostname: 'localhost',
          port,
          path: '/echo',
          method: 'GET',
          headers: {
            'Upgrade': 'websocket',
            'Connection': 'Upgrade',
            'Sec-WebSocket-Key': 'dGhlIHNhbXBsZSBub25jZQ==',
            'Sec-WebSocket-Version': '13'
          }
        }, (res) => {
          let data = ''
          res.on('data', chunk => { data += chunk })
          res.on('end', () => {
            resolve({ status: res.statusCode, body: data })
          })
        })
        req.on('error', reject)
        req.end()
      })

      strictEqual(upgradeResponse.body, 'Integration test message', 'Should echo the message through WebSocket')

      // Test regular HTTP request (should get 426 Upgrade Required)
      const httpResponse = await fetch(`http://localhost:${port}/echo`)
      strictEqual(httpResponse.status, 426, 'Should require upgrade for WebSocket app')
      const body = await httpResponse.text()
      strictEqual(body, 'Upgrade Required')
    } finally {
      server.close()
    }
  })

  await t.test('Bidirectional communication simulation', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    // Simulate a bidirectional WebSocket connection
    const req = new Request({
      method: 'GET',
      url: 'http://example.com/echo',
      websocket: true
    })

    const res = await python.handleStream(req)

    // Simulate multiple back-and-forth messages
    const messages = [
      { send: 'Message 1', expect: 'Message 1' },
      { send: 'Message 2', expect: 'Message 2' },
      { send: 'Message 3', expect: 'Message 3' }
    ]

    for (const { send, expect } of messages) {
      // Client sends message
      await req.write(send)

      // Server responds
      const chunk = await res.next()
      strictEqual(chunk.toString('utf8'), expect)
    }

    await req.end()
  })

  await t.test('Concurrent WebSocket connections', async () => {
    const python = new Python({
      docroot: fixturesDir,
      appTarget: 'websocket_app:app'
    })

    // Create multiple concurrent WebSocket connections
    const connections = []
    const numConnections = 10

    for (let i = 0; i < numConnections; i++) {
      const req = new Request({
        method: 'GET',
        url: 'http://example.com/echo',
        websocket: true
      })

      const res = python.handleStream(req)
      connections.push({ req, res, id: i })
    }

    // Wait for all connections to establish
    await Promise.all(connections.map(c => c.res))

    // Send messages on all connections concurrently
    const sendPromises = connections.map(async ({ req, res, id }) => {
      const message = `Connection ${id}`
      await req.write(message)

      const awaitedRes = await res
      const chunk = await awaitedRes.next()
      strictEqual(chunk.toString('utf8'), message)

      await req.end()
    })

    await Promise.all(sendPromises)
  })

  await t.test('WebSocket with large message payload', async () => {
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

    // Send a large message (1MB)
    const largeMessage = 'x'.repeat(1024 * 1024)
    await req.write(largeMessage)

    // Receive and verify
    const chunk = await res.next()
    strictEqual(chunk.toString('utf8').length, largeMessage.length)
    strictEqual(chunk.toString('utf8'), largeMessage)

    await req.end()
  })

  await t.test('WebSocket message buffering', async () => {
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

    // Send multiple messages quickly without waiting for responses
    const messages = ['fast1', 'fast2', 'fast3', 'fast4', 'fast5']

    for (const msg of messages) {
      await req.write(msg)
    }

    // Now read all responses
    for (const msg of messages) {
      const chunk = await res.next()
      strictEqual(chunk.toString('utf8'), msg)
    }

    await req.end()
  })
})
