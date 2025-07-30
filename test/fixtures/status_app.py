async def app(scope, receive, send):
    path = scope['path']
    status = int(path.split('/')[-1]) if path.split('/')[-1].isdigit() else 200
    
    # Read request to consume it
    await receive()
    
    await send({
        'type': 'http.response.start',
        'status': status,
        'headers': [],
    })
    
    await send({
        'type': 'http.response.body',
        'body': f'Status: {status}'.encode(),
        'more_body': False,
    })