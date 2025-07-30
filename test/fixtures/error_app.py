async def app(scope, receive, send):
    # Read request to consume it
    await receive()
    
    if scope['path'] == '/error':
        raise Exception('Test error')
    
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [],
    })
    
    await send({
        'type': 'http.response.body',
        'body': b'OK',
        'more_body': False,
    })