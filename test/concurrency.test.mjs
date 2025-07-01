import test from 'node:test';
import assert from 'node:assert';
import { Python, Request } from '../index.js';

test('Python - concurrent handleRequest calls', async (t) => {
  const python = new Python({
    docroot: './test/fixtures',
  });

  await t.test('handles multiple concurrent requests without crashes', async () => {
    const numRequests = 1000;
    const requests = [];

    // Create multiple concurrent requests
    for (let i = 0; i < numRequests; i++) {
      const promise = python.handleRequest(new Request({
        method: 'GET',
        url: `/?request=${i}`,
      }));
      requests.push(promise);
    }

    // Wait for all requests to complete
    const responses = await Promise.all(requests);

    // Verify all responses are successful
    assert.strictEqual(responses.length, numRequests);
    responses.forEach((response) => {
      assert.strictEqual(response.status, 200);
      assert.strictEqual(response.body.toString(), 'Hello, world!');
    });
  });

  await t.test('handles concurrent requests with different endpoints', async () => {
    const endpoints = [
      { url: '/', expected: 'Hello, world!' },
      { url: '/echo', expectedJson: { method: 'GET', path: '/echo', body: '', headers: {} }, appTarget: 'echo_app:app' },
      { url: '/status/201', expected: '', appTarget: 'status_app:app' },
      { url: '/error', expected: null, appTarget: 'error_app:app', shouldError: true },
    ];

    // Generate handlers for each endpoint
    for (const endpoint of endpoints) {
      if (endpoint.appTarget) {
        endpoint.handler = new Python({
          docroot: './test/fixtures',
          appTarget: endpoint.appTarget,
        });
      } else {
        endpoint.handler = python;
      }
    }

    const requests = [];

    // Create requests for different endpoints concurrently
    for (let i = 0; i < 10; i++) {
      for (const endpoint of endpoints) {
        const promise = endpoint.handler.handleRequest(new Request({
          method: 'GET',
          url: endpoint.url,
        })).then(
          response => ({ response, endpoint, error: null }),
          error => ({ response: null, endpoint, error })
        );

        requests.push(promise);
      }
    }

    // Wait for all requests
    const results = await Promise.all(requests);

    // Verify results
    results.forEach(({ response, endpoint, error }) => {
      if (endpoint.shouldError) {
        assert.ok(error, `Expected error for ${endpoint.url}`);
      } else {
        assert.ok(response, `Expected response for ${endpoint.url}`);
        if (endpoint.url === '/status/201') {
          assert.strictEqual(response.status, 201);
        } else {
          assert.strictEqual(response.status, 200);
        }
        if (endpoint.expected) {
          assert.strictEqual(response.body.toString(), endpoint.expected);
        } else if (endpoint.expectedJson) {
          const responseJson = JSON.parse(response.body.toString());
          assert.deepStrictEqual(responseJson, endpoint.expectedJson);
        }
      }
    });
  });

  await t.test('handles concurrent requests with varying response times', async () => {
    // Create a handler that uses the streaming app (which has delays)
    const streamingHandler = new Python({
      docroot: './test/fixtures',
      appTarget: 'stream_app:app',
    });

    const requests = [];

    // Mix fast and slow requests
    for (let i = 0; i < 20; i++) {
      if (i % 2 === 0) {
        // Fast request
        requests.push(
          python.handleRequest(new Request({
            method: 'GET',
            url: '/',
          }))
        );
      } else {
        // Slow request (streaming)
        requests.push(
          streamingHandler.handleRequest(new Request({
            method: 'GET',
            url: '/',
          }))
        );
      }
    }

    const start = Date.now();
    const responses = await Promise.all(requests);
    const duration = Date.now() - start;

    // All requests should complete
    assert.strictEqual(responses.length, 20);

    // Fast requests should return "Hello, world!"
    // Slow requests should return streaming data
    responses.forEach((response, index) => {
      assert.strictEqual(response.status, 200);
      if (index % 2 === 0) {
        assert.strictEqual(response.body.toString(), 'Hello, world!');
      } else {
        assert.strictEqual(response.body.toString(), 'Chunk 1\nChunk 2\nChunk 3\n');
      }
    });

    // Should complete reasonably quickly (streaming requests have 30ms delay)
    assert.ok(duration < 200, `Requests took too long: ${duration}ms`);
  });

  await t.test('handles requests with large payloads concurrently', async () => {
    // Use the echo app to test large payloads
    const echoHandler = new Python({
      docroot: './test/fixtures',
      appTarget: 'echo_app:app',
    });

    const requests = [];
    const payloadSizes = [1, 10, 100, 1000, 10000];

    for (const size of payloadSizes) {
      for (let i = 0; i < 5; i++) {
        const payload = 'x'.repeat(size);
        requests.push(
          echoHandler.handleRequest(new Request({
            method: 'POST',
            url: '/',
            body: Buffer.from(payload),
          }))
        );
      }
    }

    const responses = await Promise.all(requests);

    assert.strictEqual(responses.length, 25);
    let responseIndex = 0;
    for (const size of payloadSizes) {
      for (let i = 0; i < 5; i++) {
        const response = responses[responseIndex++];
        assert.strictEqual(response.status, 200);
        const responseJson = JSON.parse(response.body.toString());
        assert.strictEqual(responseJson.method, 'POST');
        assert.strictEqual(responseJson.path, '/');
        assert.strictEqual(responseJson.body, 'x'.repeat(size));
      }
    }
  });

  await t.test('handles handler creation and requests concurrently', async () => {
    // Create multiple handlers and use them concurrently
    const handlerPromises = [];

    for (let i = 0; i < 10; i++) {
      handlerPromises.push((async () => {
        const handler = new Python({
          docroot: './test/fixtures',
          appTarget: i % 2 === 0 ? 'main:app' : 'echo_app:app',
        });

        // Each handler makes multiple requests
        const requests = [];
        for (let j = 0; j < 10; j++) {
          requests.push(
            handler.handleRequest(new Request({
              method: 'GET',
              url: `/?handler=${i}&request=${j}`,
            }))
          );
        }

        const responses = await Promise.all(requests);
        return { handler: i, responses };
      })());
    }

    const results = await Promise.all(handlerPromises);

    assert.strictEqual(results.length, 10);
    results.forEach(({ handler, responses }) => {
      assert.strictEqual(responses.length, 10);
      responses.forEach((response) => {
        assert.strictEqual(response.status, 200);
        // Even handlers use main:app, odd use echo_app
        if (handler % 2 === 0) {
          assert.strictEqual(response.body.toString(), 'Hello, world!');
        } else {
          const responseJson = JSON.parse(response.body.toString());
          assert.strictEqual(responseJson.method, 'GET');
          assert.strictEqual(responseJson.path, `/`);
          assert.ok(responseJson.path);
        }
      });
    });
  });
});
