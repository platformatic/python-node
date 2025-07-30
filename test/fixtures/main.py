async def app(scope, receive, send):
    assert scope['type'] == 'http'
    
    # Read the request body (if any)
    request = await receive()
    assert request['type'] == 'http.request'
    
    # Send response
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            [b'content-type', b'text/plain'],
        ],
    })

    await send({
        'type': 'http.response.body',
        'body': b'Hello, world!',
        'more_body': False,
    })