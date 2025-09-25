# API Reference

Complete API documentation for `@platformatic/python-node` - A Node.js native addon for running ASGI-compatible Python applications.

## Table of Contents

- [Environment Variables](#environment-variables)
- [Classes](#classes)
  - [**`Python`**](#python)
  - [**`Request`**](#request)
  - [**`Response`**](#response)
  - [**`Headers`**](#headers)
- [Type Definitions](#type-definitions)
  - [PythonOptions](#pythonoptions)
  - [RequestOptions](#requestoptions)
  - [ResponseOptions](#responseoptions)
  - [HeaderMap](#headermap)
  - [HeaderMapValue](#headermapvalue)
  - [SocketInfo](#socketinfo)
- [Other Examples](#usage-examples)
  - [Basic ASGI Application](#basic-asgi-application)
  - [Working with Request Data](#working-with-request-data)
  - [Error Handling](#error-handling)
  - [Header Manipulation](#header-manipulation)

## Environment Variables

- **`VIRTUAL_ENV`**: Path to Python virtual environment (automatically detected)
- **`PYTHONPATH`**: Additional Python module search paths

## Classes

### Python

Main handler class for executing Python ASGI applications within Node.js.

#### Constructor

```javascript
new Python(options?: PythonOptions)
```

Creates a new Python handler instance.

**Parameters:**
- `options` *(PythonOptions, optional)*: Configuration options

**Example:**
```javascript
import { Python } from '@platformatic/python-node'

const python = new Python({
  docroot: './python-apps',
  appTarget: 'main:app'
})
```

#### Properties

##### `docroot`
- **Type:** `string`
- **Read-only**

Gets the document root directory where Python files are located.

```javascript
console.log(python.docroot) // '/path/to/python-apps'
```

#### Methods

##### `handleRequest(request)`
- **Parameters:** `request` *(Request)*: The HTTP request to process
- **Returns:** `Promise<Response>`: Promise resolving to HTTP response
- **Async:** Yes

Handles an HTTP request asynchronously using the configured Python ASGI application.

```javascript
const response = await python.handleRequest(new Request({
  method: 'GET',
  url: '/api/users'
}))

console.log(response.status) // 200
console.log(response.body.toString()) // Response body
```

##### `handleRequestSync(request)`
- **Parameters:** `request` *(Request)*: The HTTP request to process
- **Returns:** `Response`: HTTP response
- **Async:** No

Handles an HTTP request synchronously using the configured Python ASGI application.

```javascript
const response = python.handleRequestSync(new Request({
  method: 'POST',
  url: '/api/data',
  body: Buffer.from('{"key": "value"}')
}))
```

### Request

Represents an HTTP request with methods to access and modify request properties.

#### Constructor

```javascript
new Request(options?: RequestOptions)
```

Creates a new Request instance.

**Parameters:**
- `options` *(RequestOptions, optional)*: Request configuration

**Example:**
```javascript
import { Request } from '@platformatic/python-node'

const request = new Request({
  method: 'POST',
  url: 'https://example.com/api/endpoint',
  headers: {
    'Content-Type': 'application/json',
    'Authorization': 'Bearer token123'
  },
  body: Buffer.from(JSON.stringify({ message: 'Hello' }))
})
```

#### Properties

##### `method`
- **Type:** `string`
- **Read/Write**

Gets or sets the HTTP method.

```javascript
request.method = 'PUT'
console.log(request.method) // 'PUT'
```

##### `url`
- **Type:** `string`
- **Read/Write**

Gets or sets the full request URL including scheme and authority.

```javascript
request.url = 'https://api.example.com/v2/users'
console.log(request.url) // 'https://api.example.com/v2/users'
```

##### `path`
- **Type:** `string`
- **Read-only**

Gets the path portion of the URL (excluding query parameters).

```javascript
// For URL: https://example.com/api/users?page=1
console.log(request.path) // '/api/users'
```

##### `headers`
- **Type:** `Headers`
- **Read/Write**

Gets or sets the request headers.

```javascript
request.headers.set('Authorization', 'Bearer new-token')
console.log(request.headers.get('Content-Type'))
```

##### `docroot`
- **Type:** `string | null`
- **Read/Write**

Gets or sets the document root for the request.

```javascript
request.docroot = '/var/www/html'
console.log(request.docroot) // '/var/www/html'
```

##### `body`
- **Type:** `Buffer`
- **Read/Write**

Gets or sets the request body as a Buffer.

```javascript
request.body = Buffer.from('{"updated": true}')
console.log(request.body.toString()) // '{"updated": true}'
```

#### Methods

##### `toJSON()`
- **Returns:** `object`: JSON representation of the request

Converts the request to a plain JavaScript object.

```javascript
const requestData = request.toJSON()
console.log(requestData.method) // 'GET'
console.log(requestData.headers) // Header object
```

### Response

Represents an HTTP response with status, headers, and body.

#### Constructor

```javascript
new Response(options?: ResponseOptions)
```

Creates a new Response instance.

**Parameters:**
- `options` *(ResponseOptions, optional)*: Response configuration

**Example:**
```javascript
import { Response } from '@platformatic/python-node'

const response = new Response({
  status: 200,
  headers: {
    'Content-Type': 'application/json'
  },
  body: Buffer.from(JSON.stringify({ success: true }))
})
```

#### Properties

##### `status`
- **Type:** `number`
- **Read/Write**

Gets or sets the HTTP status code.

```javascript
response.status = 404
console.log(response.status) // 404
```

##### `headers`
- **Type:** `Headers`
- **Read/Write**

Gets or sets the response headers.

```javascript
response.headers.set('Cache-Control', 'max-age=3600')
console.log(response.headers.get('Content-Type'))
```

##### `body`
- **Type:** `Buffer`
- **Read/Write**

Gets or sets the response body as a Buffer.

```javascript
response.body = Buffer.from('Error occurred')
console.log(response.body.toString()) // 'Error occurred'
```

##### `log`
- **Type:** `Buffer`
- **Read-only**

Gets any log output from the response processing.

```javascript
console.log(response.log.toString()) // Log messages
```

##### `exception`
- **Type:** `string | null`
- **Read-only**

Gets any exception message from the response processing.

```javascript
if (response.exception) {
  console.error('Python error:', response.exception)
}
```

#### Methods

##### `toJSON()`
- **Returns:** `object`: JSON representation of the response

Converts the response to a plain JavaScript object.

```javascript
const responseData = response.toJSON()
console.log(responseData.status) // 200
console.log(responseData.headers) // Header object
```

### Headers

HTTP header management with support for multi-value headers.

#### Constructor

```javascript
new Headers(options?: HeaderMap)
```

Creates a new Headers instance.

**Parameters:**
- `options` *(HeaderMap, optional)*: Initial headers

**Example:**
```javascript
import { Headers } from '@platformatic/python-node'

const headers = new Headers({
  'Content-Type': 'application/json',
  'Accept': ['text/html', 'application/json'],
  'X-Custom-Header': 'CustomValue'
})
```

#### Properties

##### `size`
- **Type:** `number`
- **Read-only**

Gets the number of header entries.

```javascript
console.log(headers.size) // 3
```

#### Methods

##### `get(key)`
- **Parameters:** `key` *(string)*: Header name
- **Returns:** `string | null`: Last header value or null

Gets the last set value for a header key.

```javascript
const contentType = headers.get('Content-Type')
console.log(contentType) // 'application/json'
```

##### `getAll(key)`
- **Parameters:** `key` *(string)*: Header name
- **Returns:** `Array<string>`: All header values

Gets all values for a header key as an array.

```javascript
const acceptValues = headers.getAll('Accept')
console.log(acceptValues) // ['text/html', 'application/json']
```

##### `getLine(key)`
- **Parameters:** `key` *(string)*: Header name
- **Returns:** `string | null`: Comma-separated values or null

Gets all values for a header key as a comma-separated string.

```javascript
const acceptLine = headers.getLine('Accept')
console.log(acceptLine) // 'text/html,application/json'
```

##### `has(key)`
- **Parameters:** `key` *(string)*: Header name
- **Returns:** `boolean`: Whether header exists

Checks if a header key exists.

```javascript
console.log(headers.has('Content-Type')) // true
console.log(headers.has('Non-Existent')) // false
```

##### `set(key, value)`
- **Parameters:**
  - `key` *(string)*: Header name
  - `value` *(HeaderMapValue)*: Header value(s)
- **Returns:** `boolean`: Success status

Sets a header key/value pair, replacing any existing values.

```javascript
headers.set('Authorization', 'Bearer token123')
headers.set('Accept', ['text/html', 'application/json'])
```

##### `add(key, value)`
- **Parameters:**
  - `key` *(string)*: Header name
  - `value` *(string)*: Header value
- **Returns:** `boolean`: Success status

Adds a value to an existing header key.

```javascript
headers.add('Accept', 'text/xml')
console.log(headers.getAll('Accept')) // ['text/html', 'application/json', 'text/xml']
```

##### `delete(key)`
- **Parameters:** `key` *(string)*: Header name
- **Returns:** `boolean`: Success status

Deletes all values for a header key.

```javascript
headers.delete('X-Custom-Header')
console.log(headers.has('X-Custom-Header')) // false
```

##### `clear()`

Clears all header entries.

```javascript
headers.clear()
console.log(headers.size) // 0
```

##### `entries()`
- **Returns:** `Array<[string, string]>`: Header entries

Gets an array of [key, value] pairs for all headers.

```javascript
for (const [name, value] of headers.entries()) {
  console.log(`${name}: ${value}`)
}
```

##### `keys()`
- **Returns:** `Array<string>`: Header names

Gets an array of all header names.

```javascript
const headerNames = headers.keys()
console.log(headerNames) // ['content-type', 'accept', 'authorization']
```

##### `values()`
- **Returns:** `Array<string>`: Header values

Gets an array of all header values.

```javascript
const headerValues = headers.values()
console.log(headerValues) // ['application/json', 'text/html,application/json', 'Bearer token123']
```

##### `forEach(callback)`
- **Parameters:** `callback` *(function)*: Callback function

Executes a callback for each header entry.

```javascript
headers.forEach((value, name, headers) => {
  console.log(`${name}: ${value}`)
})
```

##### `toJSON()`
- **Returns:** `object`: JSON representation

Converts headers to a plain JavaScript object.

```javascript
const headerObj = headers.toJSON()
console.log(headerObj['content-type']) // 'application/json'
```

## Type Definitions

### PythonOptions

Configuration options for creating a Python handler.

```typescript
interface PythonOptions {
  /** Directory containing Python files */
  docroot?: string

  /** Python module and function target in "module:function" format */
  appTarget?: string
}
```

**Example:**
```javascript
const options = {
  docroot: '/path/to/python/app',
  appTarget: 'main:app'  // Import 'app' from 'main.py'
}
```

### RequestOptions

Configuration options for creating a Request.

```typescript
interface RequestOptions {
  /** HTTP method (default: 'GET') */
  method?: string

  /** Request URL (required) */
  url: string

  /** Request headers */
  headers?: Headers | HeaderMap

  /** Request body */
  body?: Buffer

  /** Socket information */
  socket?: SocketInfo

  /** Document root directory */
  docroot?: string
}
```

### ResponseOptions

Configuration options for creating a Response.

```typescript
interface ResponseOptions {
  /** HTTP status code (default: 200) */
  status?: number

  /** Response headers */
  headers?: Headers | HeaderMap

  /** Response body */
  body?: Buffer

  /** Log output */
  log?: Buffer

  /** Exception message */
  exception?: string
}
```

### HeaderMap

Type representing HTTP headers as a plain object.

```typescript
type HeaderMap = Record<string, HeaderMapValue>
```

### HeaderMapValue

Type for header values, supporting single values or arrays.

```typescript
type HeaderMapValue = string | Array<string>
```

**Examples:**
```javascript
const headers = {
  'Content-Type': 'application/json',           // Single value
  'Accept': ['text/html', 'application/json'], // Multiple values
  'Authorization': 'Bearer token123'            // Single value
}
```

### SocketInfo

Information about the network socket for a request.

```typescript
interface SocketInfo {
  /** Local IP address */
  localAddress: string

  /** Local port number */
  localPort: number

  /** Local IP family ('IPv4' or 'IPv6') */
  localFamily: string

  /** Remote IP address */
  remoteAddress: string

  /** Remote port number */
  remotePort: number

  /** Remote IP family ('IPv4' or 'IPv6') */
  remoteFamily: string
}
```

## Other Examples

### Basic ASGI Application

Create a Python ASGI app (`app.py`):

```python
async def app(scope, receive, send):
    """Basic ASGI application"""
    assert scope['type'] == 'http'

    # Read the request
    request = await receive()

    # Send response
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [[b'content-type', b'text/plain']],
    })

    await send({
        'type': 'http.response.body',
        'body': b'Hello from Python!',
        'more_body': False,
    })
```

Use it in Node.js:

```javascript
import { Python, Request } from '@platformatic/python-node'

// Create Python handler
const python = new Python({
  docroot: process.cwd(),    // Directory containing Python files
  appTarget: 'app:app'       // module:function format
})

// Handle a request
const request = new Request({
  method: 'GET',
  url: '/hello',
  headers: { 'Accept': 'text/plain' }
})

// Async handling
const response = await python.handleRequest(request)
console.log(response.status)        // 200
console.log(response.body.toString()) // 'Hello from Python!'

// Synchronous handling
const syncResponse = python.handleRequestSync(request)
```

### Working with Request Data

```javascript
// Handle POST request with JSON body
const request = new Request({
  method: 'POST',
  url: '/api/data',
  headers: {
    'Content-Type': 'application/json',
    'Authorization': 'Bearer token123'
  },
  body: Buffer.from(JSON.stringify({ name: 'John', age: 30 }))
})

const response = await python.handleRequest(request)
```

### Error Handling

```javascript
const response = await python.handleRequest(request)

// Check for Python exceptions
if (response.exception) {
  console.error('Python error:', response.exception)
}

// Check status code
if (response.status >= 400) {
  console.error('HTTP error:', response.status, response.body.toString())
}

// Access logs
if (response.log.length > 0) {
  console.log('Python logs:', response.log.toString())
}
```

### Header Manipulation

```javascript
import { Headers, Request } from '@platformatic/python-node'

// Create headers with multiple values
const headers = new Headers({
  'Accept': ['text/html', 'application/json'],
  'Accept-Language': ['en-US', 'en']
})

// Add more values
headers.add('Accept', 'text/xml')
headers.set('Authorization', 'Bearer token123')

const request = new Request({
  method: 'GET',
  url: '/api/data',
  headers: headers
})

const response = await python.handleRequest(request)

// Examine response headers
console.log('Response content type:', response.headers.get('content-type'))
console.log('All headers:', response.headers.toJSON())
```
