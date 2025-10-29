import asyncio

async def app(scope, receive, send):
    path = scope['path']
    query_string = scope.get('query_string', b'').decode('utf-8')

    # Parse simple query parameters
    params = {}
    if query_string:
        for param in query_string.split('&'):
            if '=' in param:
                key, value = param.split('=', 1)
                params[key] = value

    # Read request to consume it
    await receive()

    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [[b'content-type', b'text/plain']],
    })

    # Handle different paths
    if path == '/empty':
        # Send empty response
        await send({
            'type': 'http.response.body',
            'body': b'',
            'more_body': False,
        })
    else:
        # Send response in chunks
        # Support 'count' parameter to control number of chunks (default: 5)
        # Support 'newlines' parameter to control newlines (default: true)
        count = int(params.get('count', '5'))
        use_newlines = params.get('newlines', 'true').lower() != 'false'

        for i in range(count):
            chunk_text = f'Chunk {i + 1}'
            if use_newlines:
                chunk_text += '\n'

            await send({
                'type': 'http.response.body',
                'body': chunk_text.encode(),
                'more_body': i < count - 1,
            })
            await asyncio.sleep(0.01)  # Small delay to simulate streaming