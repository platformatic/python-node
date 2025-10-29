async def app(scope, receive, send):
    """
    WebSocket ASGI application for testing.

    Supports various test scenarios based on the path:
    - /echo: Echo back received messages
    - /uppercase: Convert text messages to uppercase
    - /close: Accept connection then immediately close
    - /ping-pong: Respond to 'ping' with 'pong'
    """

    if scope['type'] == 'websocket':
        # Accept the WebSocket connection
        await send({
            'type': 'websocket.accept',
        })

        path = scope.get('path', '/')

        if path == '/close':
            # Immediately close after accepting
            await send({
                'type': 'websocket.close',
                'code': 1000,
                'reason': 'Normal closure',
            })
            return

        # Handle messages until disconnect
        while True:
            message = await receive()

            if message['type'] == 'websocket.disconnect':
                # Client disconnected
                break

            if message['type'] == 'websocket.receive':
                text = message.get('text')
                bytes_data = message.get('bytes')

                if path == '/echo':
                    # Echo back the message
                    if text is not None:
                        await send({
                            'type': 'websocket.send',
                            'text': text,
                        })
                    elif bytes_data is not None:
                        await send({
                            'type': 'websocket.send',
                            'bytes': bytes_data,
                        })

                elif path == '/uppercase':
                    # Convert to uppercase (text only)
                    if text is not None:
                        await send({
                            'type': 'websocket.send',
                            'text': text.upper(),
                        })
                    elif bytes_data is not None:
                        # For binary, convert to uppercase if valid UTF-8
                        try:
                            text = bytes_data.decode('utf-8')
                            await send({
                                'type': 'websocket.send',
                                'text': text.upper(),
                            })
                        except UnicodeDecodeError:
                            # Send back as-is if not valid UTF-8
                            await send({
                                'type': 'websocket.send',
                                'bytes': bytes_data,
                            })

                elif path == '/ping-pong':
                    # Respond to 'ping' with 'pong'
                    if text == 'ping':
                        await send({
                            'type': 'websocket.send',
                            'text': 'pong',
                        })
                    else:
                        # Echo other messages
                        if text is not None:
                            await send({
                                'type': 'websocket.send',
                                'text': text,
                            })
                        elif bytes_data is not None:
                            await send({
                                'type': 'websocket.send',
                                'bytes': bytes_data,
                            })

                else:
                    # Default: echo
                    if text is not None:
                        await send({
                            'type': 'websocket.send',
                            'text': text,
                        })
                    elif bytes_data is not None:
                        await send({
                            'type': 'websocket.send',
                            'bytes': bytes_data,
                        })

    else:
        # Not a WebSocket request - return 426 Upgrade Required
        await send({
            'type': 'http.response.start',
            'status': 426,
            'headers': [
                (b'upgrade', b'WebSocket'),
            ],
        })
        await send({
            'type': 'http.response.body',
            'body': b'Upgrade Required',
            'more_body': False,
        })
