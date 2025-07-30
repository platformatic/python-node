# Test Fixtures

This directory contains Python ASGI applications used for testing the Python-Node integration.

## Files

### main.py
Basic ASGI application that returns a simple "Hello, world!" response. Used for testing basic request/response handling.

### echo_app.py
ASGI application that echoes back request information as JSON. Used for testing:
- Request method, path, and headers parsing
- Request body handling
- Custom response headers

### status_app.py
ASGI application that returns different HTTP status codes based on the request path. Used for testing HTTP status code handling.

### stream_app.py
ASGI application that streams the response body in chunks. Used for testing streaming responses with the `more_body` flag.

### error_app.py
ASGI application that raises an exception for certain paths. Used for testing error handling and exception propagation.

## ASGI Specification

All applications follow the ASGI 3.0 specification:
- Receive HTTP scope dictionary with request information
- Use `receive()` callable to get request body
- Use `send()` callable to send response start and body messages
- Handle async/await properly for all operations