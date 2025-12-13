use http_handler::{Request, RequestExt, Version};
use pyo3::Borrowed;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::net::SocketAddr;

use crate::asgi::{AsgiInfo, HttpVersion};

/// WebSocket connections’ scope lives as long as the socket itself - if
/// the application dies the socket should be closed, and vice-versa.
#[derive(Debug, Default)]
pub struct WebSocketConnectionScope {
  /// One of "1.1" or "2".
  http_version: HttpVersion,
  /// URL scheme portion (likely "ws" or "wss"). Optional (but must not
  /// be empty); default is "ws".
  scheme: String,
  /// HTTP request target excluding any query string, with
  /// percent-encoded sequences and UTF-8 byte sequences decoded into
  /// characters.
  path: String,
  /// The original HTTP path component, excluding any query string,
  /// unmodified from the bytes that were received by the web server.
  /// Some web server implementations may be unable to provide this.
  /// Optional; if missing defaults to None.
  raw_path: String,
  /// URL portion after the ?. Optional; if missing or None default is
  /// empty string.
  query_string: String,
  /// The root path this application is mounted at; same as SCRIPT_NAME
  /// in WSGI. Optional; if missing defaults to empty string.
  root_path: String,
  /// An iterable of [name, value] two-item iterables, where name is the
  /// header name and value is the header value. Order should be
  /// preserved from the original HTTP request; duplicates are possible
  /// and must be preserved in the message as received. Header names
  /// should be lowercased, but it is not required; servers should
  /// preserve header case on a best-effort basis. Pseudo headers
  /// (present in HTTP/2 and HTTP/3) must be removed; if :authority is
  /// present its value must be added to the start of the iterable with
  /// host as the header name or replace any existing host header already
  /// present.
  // TODO: Use a http::HeaderMap here?
  headers: Vec<(String, String)>,
  /// A two-item iterable of [host, port], where host is the remote
  /// host’s IPv4 or IPv6 address, and port is the remote port. Optional;
  /// if missing defaults to None.
  client: Option<(String, u16)>,
  /// Either a two-item iterable of [host, port], where host is the
  /// listening address for this server, and port is the integer
  /// listening port, or [path, None] where path is that of the unix
  /// socket. Optional; if missing defaults to None.
  server: Option<(String, u16)>,
  /// Subprotocols the client advertised. Optional; if missing defaults
  /// to empty list.
  subprotocols: Vec<String>,
  /// A copy of the namespace passed into the lifespan corresponding to
  /// this request. (See Lifespan Protocol). Optional; if missing the
  /// server does not support this feature.
  state: Option<Py<PyDict>>,
}

impl TryFrom<&Request> for WebSocketConnectionScope {
  type Error = PyErr;

  fn try_from(request: &Request) -> Result<Self, Self::Error> {
    // Extract HTTP version
    let http_version = match request.version() {
      Version::HTTP_10 => HttpVersion::V1_0,
      Version::HTTP_11 => HttpVersion::V1_1,
      Version::HTTP_2 => HttpVersion::V2_0,
      Version::HTTP_3 => HttpVersion::V2_0, // treat HTTP/3 as HTTP/2 for ASGI
      _ => HttpVersion::V1_1,               // default fallback
    };

    // Extract scheme from URI (typically wss or ws for WebSocket)
    let scheme = request
      .uri()
      .scheme_str()
      .map(|s| {
        if s == "https" || s == "wss" {
          "wss"
        } else {
          "ws"
        }
      })
      .unwrap_or("ws")
      .to_string();

    // Extract path
    let path = request.uri().path().to_string();

    // Extract raw path (same as path for now, as we don't have the raw bytes)
    let raw_path = path.clone();

    // Extract query string
    let query_string = request.uri().query().unwrap_or("").to_string();

    // Extract root path from DocumentRoot extension
    let root_path = request
      .document_root()
      .map(|doc_root| doc_root.path.to_string_lossy().to_string())
      .unwrap_or_default();

    // Convert headers
    let headers: Vec<(String, String)> = request
      .headers()
      .iter()
      .map(|(name, value)| {
        (
          name.as_str().to_lowercase(),
          value.to_str().unwrap_or("").to_string(),
        )
      })
      .collect();

    // Extract client and server from socket info if available
    let (client, server) = if let Some(socket_info) = request.socket_info() {
      let client = socket_info.remote.map(|addr| match addr {
        SocketAddr::V4(v4) => (v4.ip().to_string(), v4.port()),
        SocketAddr::V6(v6) => (v6.ip().to_string(), v6.port()),
      });
      let server = socket_info.local.map(|addr| match addr {
        SocketAddr::V4(v4) => (v4.ip().to_string(), v4.port()),
        SocketAddr::V6(v6) => (v6.ip().to_string(), v6.port()),
      });
      (client, server)
    } else {
      (None, None)
    };

    // Extract subprotocols from Sec-WebSocket-Protocol header
    let subprotocols: Vec<String> = request
      .headers()
      .get("sec-websocket-protocol")
      .and_then(|h| h.to_str().ok())
      .map(|protocols| protocols.split(',').map(|p| p.trim().to_string()).collect())
      .unwrap_or_default();

    Ok(WebSocketConnectionScope {
      http_version,
      scheme,
      path,
      raw_path,
      query_string,
      root_path,
      headers,
      client,
      server,
      subprotocols,
      state: None,
    })
  }
}

impl<'py> IntoPyObject<'py> for WebSocketConnectionScope {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    dict.set_item("type", "websocket")?;
    dict.set_item("asgi", AsgiInfo::new("3.0", "2.5").into_pyobject(py)?)?;
    dict.set_item("http_version", self.http_version.into_pyobject(py)?)?;
    dict.set_item("scheme", self.scheme)?;
    dict.set_item("path", self.path)?;
    dict.set_item("raw_path", self.raw_path)?;
    dict.set_item("query_string", self.query_string)?;
    dict.set_item("root_path", self.root_path)?;
    dict.set_item("headers", self.headers.into_pyobject(py)?)?;
    if let Some((host, port)) = self.client {
      dict.set_item("client", (&host, port).into_pyobject(py)?)?;
    } else {
      dict.set_item("client", py.None())?;
    }
    if let Some((host, port)) = self.server {
      dict.set_item("server", (&host, port).into_pyobject(py)?)?;
    } else {
      dict.set_item("server", py.None())?;
    }
    dict.set_item("subprotocols", self.subprotocols.into_pyobject(py)?)?;
    dict.set_item("state", self.state)?;
    Ok(dict)
  }
}

//
// WebSocket Scope
//

/// WebSocket Scope messages given to `receive()` function of an ASGI application.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WebSocketReceiveMessage {
  /// Sent to the application when the client initially opens a connection
  /// and is about to finish the WebSocket handshake.
  ///
  /// This message must be responded to with either an Accept message or a
  /// Close message before the socket will pass websocket.receive messages.
  /// The protocol server must send this message during the handshake phase
  /// of the WebSocket and not complete the handshake until it gets a reply,
  /// returning HTTP status code 403 if the connection is denied.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#connect-receive-event
  Connect,
  /// Sent to the application when a data message is received from the client.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#receive-receive-event
  Receive {
    /// The message content, if it was binary mode, or None. Optional; if
    /// missing, it is equivalent to None.
    bytes: Option<Vec<u8>>,
    /// The message content, if it was text mode, or None. Optional; if
    /// missing, it is equivalent to None.
    text: Option<String>,
  },
  /// Sent to the application when either connection to the client is lost,
  /// either from the client closing the connection, the server closing the
  /// connection, or loss of the socket.
  ///
  /// Once you have received this event, you should expect future calls to
  /// send() to raise an exception, as described below. However, if you have
  /// highly concurrent code, you may find calls to send() erroring slightly
  /// before you receive this event.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#disconnect-receive-event-ws
  Disconnect {
    /// The WebSocket close code, as per the WebSocket spec. If no code was
    /// received in the frame from the client, the server should set this
    /// to 1005 (the default value in the WebSocket specification).
    code: Option<u16>,
    /// A reason given for the disconnect, can be any string. Optional; if
    /// missing or None default is empty string.
    reason: Option<String>,
  },
}

impl<'py> IntoPyObject<'py> for WebSocketReceiveMessage {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    match self {
      WebSocketReceiveMessage::Connect => {
        dict.set_item("type", "websocket.connect")?;
      }
      WebSocketReceiveMessage::Receive { bytes, text } => {
        dict.set_item("type", "websocket.receive")?;
        dict.set_item("bytes", bytes)?;
        dict.set_item("text", text)?;
      }
      WebSocketReceiveMessage::Disconnect { code, reason } => {
        dict.set_item("type", "websocket.disconnect")?;
        dict.set_item("code", code)?;
        dict.set_item("reason", reason)?;
      }
    }
    Ok(dict)
  }
}

/// WebSocket Scope messages given to the `send()` function by an ASGI application.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WebSocketSendMessage {
  /// Sent by the application when it wishes to accept an incoming connection.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#accept-send-event
  Accept {
    /// The subprotocol the server wishes to accept. Optional; if missing
    /// defaults to None.
    subprotocol: Option<String>,
    /// An iterable of [name, value] two-item iterables, where name is the
    /// header name, and value is the header value. Order must be preserved
    /// in the HTTP response. Header names must be lowercased. Must not
    /// include a header named sec-websocket-protocol; use the subprotocol
    /// key instead. Optional; if missing defaults to an empty list. Added
    /// in spec version 2.1. Pseudo headers (present in HTTP/2 and HTTP/3)
    /// must not be present.
    headers: Vec<(String, String)>,
  },
  /// Sent by the application to send a data message to the client.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#send-send-event
  Send {
    /// Binary message content, or None. Optional; if missing, it is
    /// equivalent to None.
    bytes: Option<Vec<u8>>,
    /// Text message content, or None. Optional; if missing, it is
    /// equivalent to None.
    text: Option<String>,
  },
  /// Sent by the application to tell the server to close the connection.
  ///
  /// If this is sent before the socket is accepted, the server must close
  /// the connection with a HTTP 403 error code (Forbidden), and not complete
  /// the WebSocket handshake; this may present on some browsers as a
  /// different WebSocket error code (such as 1006, Abnormal Closure).
  ///
  /// If this is sent after the socket is accepted, the server must close the
  /// socket with the close code passed in the message (or 1000 if none is
  /// specified).
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#close-send-event
  Close {
    /// The WebSocket close code, as per the WebSocket spec. Optional; if
    /// missing defaults to 1000.
    code: Option<u16>,
    /// A reason given for the closure, can be any string. Optional; if
    /// missing or None default is empty string.
    reason: Option<String>,
  },
}

impl<'a, 'py> FromPyObject<'a, 'py> for WebSocketSendMessage {
  type Error = PyErr;

  fn extract(ob: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
    let dict = ob.cast::<PyDict>()?;
    let message_type = dict.get_item("type")?.ok_or_else(|| {
      PyValueError::new_err("Missing 'type' key in WebSocket send message dictionary")
    })?;

    let message_type: String = message_type.extract()?;
    match message_type.as_str() {
      "websocket.accept" => {
        let subprotocol: Option<String> = dict
          .get_item("subprotocol")?
          .map_or(Ok(None), |v| v.extract())?;

        let headers: Vec<(String, String)> = dict
          .get_item("headers")?
          .map_or(Ok(vec![]), |v| v.extract())?;

        Ok(WebSocketSendMessage::Accept {
          subprotocol,
          headers,
        })
      }
      "websocket.send" => {
        let bytes: Option<Vec<u8>> = dict.get_item("bytes")?.map_or(Ok(None), |v| v.extract())?;

        let text: Option<String> = dict.get_item("text")?.map_or(Ok(None), |v| v.extract())?;

        // One of bytes or text must be provided
        if bytes.is_none() && text.is_none() {
          return Err(PyValueError::new_err(
            "At least one of 'bytes' or 'text' must be provided in WebSocket send message",
          ));
        }

        Ok(WebSocketSendMessage::Send { bytes, text })
      }
      "websocket.close" => {
        let code: Option<u16> = dict.get_item("code")?.map_or(Ok(None), |v| v.extract())?;

        let reason: Option<String> = dict.get_item("reason")?.map_or(Ok(None), |v| v.extract())?;

        Ok(WebSocketSendMessage::Close { code, reason })
      }
      _ => Err(PyValueError::new_err(format!(
        "Unknown WebSocket send message type: {message_type}"
      ))),
    }
  }
}

/// An exception that can occur when sending WebSocket messages.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WebSocketSendException {
  Disconnected,
}

#[cfg(test)]
mod tests {
  use super::*;

  macro_rules! dict_get {
    ($dict:expr, $key:expr) => {
      $dict
        .get_item($key)
        .expect(&("Failed to get ".to_owned() + stringify!($key)))
        .expect(&("Item \"".to_owned() + stringify!($key) + "\" not found"))
    };
  }

  macro_rules! dict_extract {
    ($dict:expr, $key:expr, $type:ty) => {
      dict_get!($dict, $key)
        .extract::<$type>()
        .expect(&("Unable to convert to ".to_owned() + stringify!($type)))
    };
  }

  #[test]
  fn test_websocket_connection_scope_into_pyobject() {
    Python::attach(|py| {
      let scope = WebSocketConnectionScope {
        http_version: HttpVersion::V2_0,
        scheme: "ws".to_string(),
        path: "/test".to_string(),
        raw_path: "/test".to_string(),
        query_string: "param=value".to_string(),
        root_path: "/".to_string(),
        headers: vec![("host".to_string(), "example.com".to_string())],
        client: Some(("client".to_string(), 1234)),
        server: Some(("server".to_string(), 5678)),
        subprotocols: vec!["chat".to_string()],
        state: Some(PyDict::new(py).unbind()),
      };

      let dict = scope.into_pyobject(py).unwrap();
      assert_eq!(dict_extract!(dict, "type", String), "websocket".to_string());
      assert_eq!(
        dict_extract!(dict, "http_version", String),
        "2.0".to_string()
      );
      assert_eq!(dict_extract!(dict, "scheme", String), "ws".to_string());
      assert_eq!(dict_extract!(dict, "path", String), "/test".to_string());
      assert_eq!(dict_extract!(dict, "raw_path", String), "/test".to_string());
      assert_eq!(
        dict_extract!(dict, "query_string", String),
        "param=value".to_string()
      );
      assert_eq!(dict_extract!(dict, "root_path", String), "/".to_string());
      assert_eq!(
        dict_extract!(dict, "headers", Vec<(String, String)>),
        vec![("host".to_string(), "example.com".to_string())]
      );
      assert_eq!(
        dict_extract!(dict, "client", (String, i16)),
        ("client".into(), 1234)
      );
      assert_eq!(
        dict_extract!(dict, "server", (String, i16)),
        ("server".into(), 5678)
      );
      assert_eq!(
        dict_extract!(dict, "subprotocols", Vec<String>),
        vec!["chat".to_string()]
      );
      assert!(!dict_get!(dict, "state").is_none());
    });
  }

  #[test]
  fn test_websocket_receive_message_into_pyobject() {
    Python::attach(|py| {
      let connect_msg = WebSocketReceiveMessage::Connect;
      let dict = connect_msg.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(dict, "type", String),
        "websocket.connect".to_string()
      );

      let receive_msg = WebSocketReceiveMessage::Receive {
        bytes: Some(vec![1, 2, 3]),
        text: Some("Hello".to_string()),
      };
      let dict = receive_msg.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(dict, "type", String),
        "websocket.receive".to_string()
      );
      assert_eq!(
        dict_extract!(dict, "bytes", Option<Vec<u8>>),
        Some(vec![1, 2, 3])
      );
      assert_eq!(
        dict_extract!(dict, "text", Option<String>),
        Some("Hello".to_string())
      );

      let disconnect_msg = WebSocketReceiveMessage::Disconnect {
        code: Some(1000),
        reason: Some("Normal Closure".to_string()),
      };
      let dict = disconnect_msg.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(dict, "type", String),
        "websocket.disconnect".to_string()
      );
      assert_eq!(dict_extract!(dict, "code", Option<u16>), Some(1000));
      assert_eq!(
        dict_extract!(dict, "reason", Option<String>),
        Some("Normal Closure".to_string())
      );
    });
  }

  #[test]
  fn test_websocket_send_message_from_pyobject() {
    Python::attach(|py| {
      let dict = PyDict::new(py);
      dict.set_item("type", "websocket.accept").unwrap();
      dict.set_item("subprotocol", "chat").unwrap();
      dict
        .set_item(
          "headers",
          vec![("host".to_string(), "example.com".to_string())],
        )
        .unwrap();

      let msg: WebSocketSendMessage = dict.extract().unwrap();
      assert_eq!(
        msg,
        WebSocketSendMessage::Accept {
          subprotocol: Some("chat".to_string()),
          headers: vec![("host".to_string(), "example.com".to_string())],
        }
      );

      let dict = PyDict::new(py);
      dict.set_item("type", "websocket.send").unwrap();
      dict.set_item("bytes", vec![1, 2, 3]).unwrap();
      dict.set_item("text", "Hello").unwrap();

      let msg: WebSocketSendMessage = dict.extract().unwrap();
      assert_eq!(
        msg,
        WebSocketSendMessage::Send {
          bytes: Some(vec![1, 2, 3]),
          text: Some("Hello".to_string()),
        }
      );

      let dict = PyDict::new(py);
      dict.set_item("type", "websocket.close").unwrap();
      dict.set_item("code", 1000).unwrap();
      dict.set_item("reason", "Normal Closure").unwrap();

      let msg: WebSocketSendMessage = dict.extract().unwrap();
      assert_eq!(
        msg,
        WebSocketSendMessage::Close {
          code: Some(1000),
          reason: Some("Normal Closure".to_string()),
        }
      );
    });
  }
}
