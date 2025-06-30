import { ok, strictEqual } from 'node:assert/strict'
import { test } from 'node:test'

import { Python, Request } from '../index.js'

test('Python', async t => {
  await t.test('constructor', () => {
    const python = new Python({
      docroot: process.cwd(),
      appTarget: 'main:app'
    })

    const request = new Request({
      method: 'GET',
      uri: '/test.php',
      headers: {
        'Content-Type': 'application/json',
        'X-Custom-Header': 'CustomValue'
      }
    })

    const response = python.handleRequestSync(request)

    ok(python instanceof Python, 'Python should be defined')
    strictEqual(python.docroot, process.cwd(), 'should set docroot correctly')
  })
})
