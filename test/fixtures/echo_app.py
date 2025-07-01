import json

async def app(scope, receive, send):
    assert scope['type'] == 'http'
    
    # Collect request info
    method = scope['method']
    path = scope['path']
    headers = {k: v for k, v in scope['headers']}
    
    # Read request body
    body_parts = []
    while True:
        message = await receive()
        if message['type'] == 'http.request':
            body = message.get('body', b'')
            if body:
                body_parts.append(body)
            if not message.get('more_body', False):
                break
    
    request_body = b''.join(body_parts)
    
    # Send response
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            [b'content-type', b'application/json'],
            [b'x-echo-method', method.encode()],
            [b'x-echo-path', path.encode()],
        ],
    })
    
    response_data = {
        'method': method,
        'path': path,
        'body': request_body.decode('utf-8'),
        'headers': {k if isinstance(k, str) else k.decode(): v if isinstance(v, str) else v.decode() for k, v in headers.items()}
    }
    
    await send({
        'type': 'http.response.body',
        'body': json.dumps(response_data).encode('utf-8'),
        'more_body': False,
    })