async def app(scope, receive, send):
    # Read request to consume it
    await receive()

    # Send response start
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [],
    })

    if scope['path'] == '/error-during-stream':
        # Send some chunks before error
        await send({
            'type': 'http.response.body',
            'body': b'Chunk 1\n',
            'more_body': True,
        })

        await send({
            'type': 'http.response.body',
            'body': b'Chunk 2\n',
            'more_body': True,
        })

        # Now raise an error during streaming
        raise Exception('Error during streaming')

    # Normal response
    await send({
        'type': 'http.response.body',
        'body': b'OK',
        'more_body': False,
    })
