async def app(scope, receive, send):
    assert scope['type'] == 'http'

    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            [b'content-type', b'text/plain'],
        ],
    })

    request = await receive()
    print("Received request:", request)

    await send({
        'type': 'http.response.body',
        'body': b'Hello, world!',
    })

print("Starting ASGI application.")
