# @platformatic/python-node

A high-performance Node.js native addon that enables running **ASGI-compatible Python applications** directly within Node.js environments. The Node.js host can communicate directly with the Python guest app without any network involved, allowing near-zero communcation overhead.

This module provides a bridge between Node.js and Python, allowing you to:

- **Run Python ASGI applications** (FastAPI, Starlette, Django ASGI, etc.) inside Node.js processes
- **Handle HTTP requests** with ASGI 3.0 protocol support
- **Process requests concurrently** using async Python code execution
- **Integrate Python services** into existing Node.js applications seamlessly
- **Support virtual environments** automatically for proper Python dependency isolation

The module implements an ASGI server that translates between Node.js HTTP requests and Python ASGI applications, enabling you to leverage Python's rich ecosystem within Node.js applications.

### Key Features

- **ASGI 3.0 Support**: HTTP and Lifespan protocols (WebSocket support planned)
- **Async/Sync Methods**: Both `handleRequest()` and `handleRequestSync()`
- **Virtual Environment Detection**: Automatic Python environment discovery
- **Cross-Platform**: Native binaries for macOS and Linux
- **High Performance**: Rust-based implementation with minimal overhead

### Requirements

- **Node.js**: ≥ 20.0.0
- **Python**: ≥ 3.8 with asyncio support
- **System**: macOS (arm64, x64) or Linux (x64-gnu)

The module automatically detects and uses Python virtual environments via the `VIRTUAL_ENV` environment variable.

## Installation

```bash
npm install @platformatic/python-node
```

## API Reference

For complete API documentation, see [REFERENCE.md](./REFERENCE.md).

## Basic Usage

```python
# fastapi_app.py
from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()

class Item(BaseModel):
    name: str
    price: float

@app.post("/items/")
async def create_item(item: Item):
    return {"name": item.name, "price": item.price}
```

```javascript
import { Python, Request } from '@platformatic/python-node'

const python = new Python({
  docroot: './python-apps',
  appTarget: 'fastapi_app:app'
})

// POST with JSON body
const response = await python.handleRequest(new Request({
  method: 'POST',
  url: '/items/',
  headers: {
    'Content-Type': 'application/json'
  },
  body: Buffer.from(JSON.stringify({
    name: 'Widget',
    price: 29.99
  }))
}))

const result = JSON.parse(response.body.toString())
console.log(result) // { name: 'Widget', price: 29.99 }
```
