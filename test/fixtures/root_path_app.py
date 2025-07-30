async def app(scope, receive, send):
    assert scope['type'] == 'http'
    
    # Read request to consume it
    await receive()
    
    # Get the root_path from scope
    root_path = scope.get('root_path', '')
    
    # Send response
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            [b'content-type', b'text/plain'],
            [b'x-root-path', root_path.encode() if root_path else b''],
        ],
    })

    await send({
        'type': 'http.response.body',
        'body': f'Root path: {root_path}'.encode(),
        'more_body': False,
    })