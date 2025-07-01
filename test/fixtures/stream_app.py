import asyncio

async def app(scope, receive, send):
    # Read request to consume it
    await receive()
    
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [[b'content-type', b'text/plain']],
    })
    
    # Send response in chunks
    for i in range(3):
        await send({
            'type': 'http.response.body',
            'body': f'Chunk {i + 1}\n'.encode(),
            'more_body': i < 2,
        })
        await asyncio.sleep(0.01)  # Small delay to simulate streaming