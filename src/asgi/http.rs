use http_handler::{Request, RequestExt, Version};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use std::net::SocketAddr;

use crate::asgi::{AsgiInfo, HttpMethod, HttpVersion};

/// HTTP connections have a single-request connection scope - that is,
/// your application will be called at the start of the request, and will
/// last until the end of that specific request, even if the underlying
/// socket is still open and serving multiple requests.
///
/// If you hold a response open for long-polling or similar, the connection
/// scope will persist until the response closes from either the client or
/// server side.
#[derive(Debug)]
pub struct HttpConnectionScope {
  /// One of "1.0", "1.1" or "2".
  http_version: HttpVersion,
  /// The HTTP method name, uppercased.
  method: HttpMethod,
  /// URL scheme portion (likely "http" or "https"). Optional (but must
  /// not be empty); default is "http".
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
  /// URL portion after the ?, percent-encoded.
  query_string: String,
  /// The root path this application is mounted at; same as SCRIPT_NAME
  /// in WSGI. Optional; if missing defaults to "".
  root_path: String,
  /// An iterable of [name, value] two-item iterables, where name is the
  /// header name, and value is the header value. Order of header values
  /// must be preserved from the original HTTP request; order of header
  /// names is not important. Duplicates are possible and must be
  /// preserved in the message as received. Header names should be
  /// lowercased, but it is not required; servers should preserve header
  /// case on a best-effort basis. Pseudo headers (present in HTTP/2 and
  /// HTTP/3) must be removed; if :authority is present its value must be
  /// added to the start of the iterable with host as the header name or
  /// replace any existing host header already present.
  // TODO: Use a http::HeaderMap here?
  headers: Vec<(String, String)>,
  /// A two-item iterable of [host, port], where host is the remote
  /// host’s IPv4 or IPv6 address, and port is the remote port as an
  /// integer. Optional; if missing defaults to None.
  client: Option<(String, u16)>,
  /// Either a two-item iterable of [host, port], where host is the
  /// listening address for this server, and port is the integer
  /// listening port, or [path, None] where path is that of the unix
  /// socket. Optional; if missing defaults to None.
  server: Option<(String, u16)>,
  /// A copy of the namespace passed into the lifespan corresponding to
  /// this request. (See Lifespan Protocol). Optional; if missing the
  /// server does not support this feature.
  state: Option<Py<PyDict>>,
}

impl TryFrom<&Request> for HttpConnectionScope {
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

    // Extract method
    let method = request.method().try_into().map_err(PyValueError::new_err)?;

    // Extract scheme from URI or default to http
    let scheme = request.uri().scheme_str().unwrap_or("http").to_string();

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

    Ok(HttpConnectionScope {
      http_version,
      method,
      scheme,
      path,
      raw_path,
      query_string,
      root_path,
      headers,
      client,
      server,
      state: None,
    })
  }
}

impl<'py> IntoPyObject<'py> for HttpConnectionScope {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    dict.set_item("type", "http")?;
    dict.set_item("asgi", AsgiInfo::new("3.0", "2.5").into_pyobject(py)?)?;
    dict.set_item("http_version", self.http_version.into_pyobject(py)?)?;
    dict.set_item("method", self.method.into_pyobject(py)?)?;
    dict.set_item("scheme", self.scheme)?;
    dict.set_item("path", self.path)?;
    dict.set_item("raw_path", self.raw_path.as_bytes())?;
    dict.set_item("query_string", self.query_string.as_bytes())?;
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
    dict.set_item("state", self.state)?;
    Ok(dict)
  }
}

//
// HTTP Scope
//

/// HTTP Scope messages given to `receive()` function of an ASGI application.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HttpReceiveMessage {
  /// Sent to the application to indicate an incoming request. Most of the
  /// request information is in the connection scope; the body message serves
  /// as a way to stream large incoming HTTP bodies in chunks, and as a
  /// trigger to actually run request code (as you should not trigger on a
  /// connection opening alone).
  ///
  /// Note that if the request is being sent using Transfer-Encoding:
  /// chunked, the server is responsible for handling this encoding. The
  /// http.request messages should contain just the decoded contents of each
  /// chunk.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#http-request
  Request {
    /// Body of the request. Optional; if missing defaults to b"". If
    /// more_body is set, treat as start of body and concatenate on further
    /// chunks.
    body: Vec<u8>,
    /// Signifies if there is additional content to come (as part of a
    /// Request message). If True, the consuming application should wait
    /// until it gets a chunk with this set to False. If False, the request
    /// is complete and should be processed. Optional; if missing defaults
    /// to False.
    // TODO: Use this for streaming large bodies.
    more_body: bool,
  },
  /// Sent to the application if receive is called after a response has been
  /// sent or after the HTTP connection has been closed. This is mainly
  /// useful for long-polling, where you may want to trigger cleanup code if
  /// the connection closes early.
  ///
  /// Once you have received this event, you should expect future calls to
  /// send() to raise an exception, as described above. However, if you have
  /// highly concurrent code, you may find calls to send() erroring slightly
  /// before you receive this event.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#disconnect-receive-event
  Disconnect,
}

impl<'py> IntoPyObject<'py> for HttpReceiveMessage {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    match self {
      HttpReceiveMessage::Request { body, more_body } => {
        dict.set_item("type", "http.request")?;
        dict.set_item("body", body)?;
        dict.set_item("more_body", more_body)?;
      }
      HttpReceiveMessage::Disconnect => {
        dict.set_item("type", "http.disconnect")?;
      }
    }
    Ok(dict)
  }
}

/// Http Scope messages given to the `send()` function by an ASGI application.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HttpSendMessage {
  /// Sent by the application to start sending a response to the client.
  /// Needs to be followed by at least one response content message.
  ///
  /// Protocol servers need not flush the data generated by this event to the
  /// send buffer until the first Response Body event is processed. This may
  /// give them more leeway to replace the response with an error response in
  /// case internal errors occur while handling the request.
  ///
  /// You may send a Transfer-Encoding header in this message, but the server
  /// must ignore it. Servers handle Transfer-Encoding themselves, and may
  /// opt to use Transfer-Encoding: chunked if the application presents a
  /// response that has no Content-Length set.
  ///
  /// Note that this is not the same as Content-Encoding, which the
  /// application still controls, and which is the appropriate place to set
  /// gzip or other compression flags.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#response-start-send-event
  HttpResponseStart {
    /// HTTP status code.
    status: u16,
    /// An iterable of [name, value] two-item iterables, where name is the
    /// header name, and value is the header value. Order must be preserved
    /// in the HTTP response. Header names must be lowercased. Optional; if
    /// missing defaults to an empty list. Pseudo headers (present in
    /// HTTP/2 and HTTP/3) must not be present.
    headers: Vec<(String, String)>,
    /// Signifies if the application will send trailers. If True, the
    /// server must wait until it receives a "http.response.trailers"
    /// message after the Response Body event. Optional; if missing
    /// defaults to False.
    trailers: bool,
  },
  /// Continues sending a response to the client. Protocol servers must flush
  /// any data passed to them into the send buffer before returning from a
  /// send call. If more_body is set to False, and the server is not
  /// expecting Response Trailers this will complete the response.
  ///
  /// - https://asgi.readthedocs.io/en/latest/specs/www.html#response-body-send-event
  HttpResponseBody {
    /// HTTP body content. Concatenated onto any previous body values sent
    /// in this connection scope. Optional; if missing defaults to b"".
    body: Vec<u8>,
    /// Signifies if there is additional content to come (as part of a
    /// Response Body message). If False, and the server is not expecting
    /// Response Trailers response will be taken as complete and closed,
    /// and any further messages on the channel will be ignored. Optional;
    /// if missing defaults to False.
    more_body: bool,
  },
}

impl<'py> FromPyObject<'py> for HttpSendMessage {
  fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
    let dict = ob.downcast::<PyDict>()?;
    let message_type = dict
      .get_item("type")?
      .ok_or_else(|| PyValueError::new_err("Missing 'type' key in HTTP send message dictionary"))?;

    let message_type: String = message_type.extract()?;
    match message_type.as_str() {
      "http.response.start" => {
        let status: u16 = dict
          .get_item("status")?
          .ok_or_else(|| {
            PyValueError::new_err("Missing 'status' key in HTTP response start message")
          })?
          .extract()?;

        let headers_py = dict.get_item("headers")?.ok_or_else(|| {
          PyValueError::new_err("Missing 'headers' key in HTTP response start message")
        })?;

        // Convert headers from list of lists to vec of tuples
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Ok(headers_list) = headers_py.downcast::<pyo3::types::PyList>() {
          for item in headers_list.iter() {
            if let Ok(header_pair) = item.downcast::<pyo3::types::PyList>() {
              if header_pair.len() == 2 {
                let name = header_pair.get_item(0)?;
                let value = header_pair.get_item(1)?;

                // Convert bytes to string
                let name_str = if let Ok(bytes) = name.downcast::<pyo3::types::PyBytes>() {
                  String::from_utf8_lossy(bytes.as_bytes()).to_string()
                } else {
                  name.extract::<String>()?
                };

                let value_str = if let Ok(bytes) = value.downcast::<pyo3::types::PyBytes>() {
                  String::from_utf8_lossy(bytes.as_bytes()).to_string()
                } else {
                  value.extract::<String>()?
                };

                headers.push((name_str, value_str));
              }
            }
          }
        }

        let trailers: bool = dict
          .get_item("trailers")?
          .map_or(Ok(false), |v| v.extract())?;

        Ok(HttpSendMessage::HttpResponseStart {
          status,
          headers,
          trailers,
        })
      }
      "http.response.body" => {
        let body: Vec<u8> = dict.get_item("body")?.map_or(Ok(vec![]), |v| v.extract())?;

        let more_body: bool = dict
          .get_item("more_body")?
          .map_or(Ok(false), |v| v.extract())?;

        Ok(HttpSendMessage::HttpResponseBody { body, more_body })
      }
      _ => Err(PyValueError::new_err(format!(
        "Unknown HTTP send message type: {message_type}"
      ))),
    }
  }
}

/// An exception that can occur when sending HTTP messages.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HttpSendException {
  Disconnected,
}

#[cfg(test)]
mod tests {
  use super::*;
  use http_handler::{Method, Version, request::Builder};
  use http_handler::{RequestExt, extensions::DocumentRoot};
  use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
  };

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
  fn test_http_connection_scope_from_request() {
    // Create a test request with various headers and extensions
    let mut request = Builder::new()
      .method(Method::POST)
      .uri("https://example.com:8443/api/v1/users?sort=name&limit=10")
      .header("content-type", "application/json")
      .header("authorization", "Bearer token123")
      .header("user-agent", "test-client/1.0")
      .header("x-custom-header", "custom-value")
      .body(bytes::BytesMut::from("request body"))
      .unwrap();

    // Set socket info extension
    let local_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8443);
    let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 12345);
    request.set_socket_info(http_handler::extensions::SocketInfo::new(
      Some(local_addr),
      Some(remote_addr),
    ));

    // Set document root extension
    let doc_root = PathBuf::from("/var/www/html");
    request.set_document_root(DocumentRoot {
      path: doc_root.clone(),
    });

    // Convert to ASGI scope
    let scope: HttpConnectionScope = (&request)
      .try_into()
      .expect("Failed to convert request to HttpConnectionScope");

    // Verify HTTP version
    assert_eq!(scope.http_version, HttpVersion::V1_1);

    // Verify method
    assert_eq!(scope.method, HttpMethod::Post);

    // Verify scheme
    assert_eq!(scope.scheme, "https");

    // Verify path
    assert_eq!(scope.path, "/api/v1/users");

    // Verify raw_path (should be same as path in this implementation)
    assert_eq!(scope.raw_path, "/api/v1/users");

    // Verify query_string
    assert_eq!(scope.query_string, "sort=name&limit=10");

    // Verify root_path from DocumentRoot extension
    assert_eq!(scope.root_path, doc_root.to_string_lossy());

    // Verify headers (should be lowercased)
    let expected_headers = vec![
      ("content-type".to_string(), "application/json".to_string()),
      ("authorization".to_string(), "Bearer token123".to_string()),
      ("user-agent".to_string(), "test-client/1.0".to_string()),
      ("x-custom-header".to_string(), "custom-value".to_string()),
    ];
    assert_eq!(scope.headers, expected_headers);

    // Verify client socket info
    assert_eq!(scope.client, Some(("192.168.1.100".to_string(), 12345)));

    // Verify server socket info
    assert_eq!(scope.server, Some(("127.0.0.1".to_string(), 8443)));

    // Verify state is None (not set)
    assert!(scope.state.is_none());
  }

  #[test]
  fn test_http_connection_scope_from_request_minimal() {
    // Test with minimal request (no extensions, no headers)
    let request = Builder::new()
      .method(Method::GET)
      .uri("/")
      .body(bytes::BytesMut::new())
      .unwrap();

    let scope: HttpConnectionScope = (&request)
      .try_into()
      .expect("Failed to convert request to HttpConnectionScope");

    assert_eq!(scope.http_version, HttpVersion::V1_1);
    assert_eq!(scope.method, HttpMethod::Get);
    assert_eq!(scope.scheme, "http"); // default scheme
    assert_eq!(scope.path, "/");
    assert_eq!(scope.raw_path, "/");
    assert_eq!(scope.query_string, "");
    assert_eq!(scope.root_path, ""); // no DocumentRoot extension
    assert_eq!(scope.headers, vec![]); // no headers
    assert_eq!(scope.client, None); // no socket info
    assert_eq!(scope.server, None); // no socket info
    assert!(scope.state.is_none());
  }

  #[test]
  fn test_http_connection_scope_from_request_http2() {
    // Test HTTP/2 version handling
    let request = Builder::new()
      .method(Method::PUT)
      .uri("http://api.example.com/resource/123")
      .version(Version::HTTP_2)
      .body(bytes::BytesMut::new())
      .unwrap();

    let scope: HttpConnectionScope = (&request)
      .try_into()
      .expect("Failed to convert request to HttpConnectionScope");

    assert_eq!(scope.http_version, HttpVersion::V2_0);
    assert_eq!(scope.method, HttpMethod::Put);
    assert_eq!(scope.scheme, "http");
    assert_eq!(scope.path, "/resource/123");
  }

  #[test]
  fn test_http_connection_scope_into_pyobject() {
    Python::initialize();
    Python::attach(|py| {
      let scope = HttpConnectionScope {
        http_version: HttpVersion::V1_1,
        method: HttpMethod::Get,
        scheme: "http".to_string(),
        path: "".to_string(),
        raw_path: "".to_string(),
        query_string: "".to_string(),
        root_path: "".to_string(),
        headers: vec![],
        client: None,
        server: None,
        state: None,
      };
      let py_scope = scope.into_pyobject(py).unwrap();

      assert_eq!(dict_extract!(py_scope, "type", String), "http".to_string());
      assert_eq!(
        dict_extract!(py_scope, "asgi", AsgiInfo),
        AsgiInfo {
          version: "3.0".into(),
          spec_version: "2.5".into()
        }
      );
      assert_eq!(
        dict_extract!(py_scope, "http_version", HttpVersion),
        HttpVersion::V1_1
      );
      assert_eq!(
        dict_extract!(py_scope, "method", HttpMethod),
        HttpMethod::Get
      );
      assert_eq!(dict_extract!(py_scope, "scheme", String), "http");
      assert_eq!(dict_extract!(py_scope, "path", String), "");
      assert_eq!(dict_extract!(py_scope, "raw_path", Vec<u8>), b"");
      assert_eq!(dict_extract!(py_scope, "query_string", Vec<u8>), b"");
      assert_eq!(dict_extract!(py_scope, "root_path", String), "");
      assert_eq!(
        dict_extract!(py_scope, "headers", Vec<(String, String)>),
        vec![]
      );
      assert!(dict_get!(py_scope, "client").is_none());
      assert!(dict_get!(py_scope, "server").is_none());
      assert!(dict_get!(py_scope, "state").is_none());
    });
  }

  #[test]
  fn test_http_receive_message_into_pyobject() {
    Python::initialize();
    Python::attach(|py| {
      let message = HttpReceiveMessage::Request {
        body: vec![1, 2, 3],
        more_body: true,
      };
      let py_message = message.into_pyobject(py).unwrap();

      assert_eq!(
        dict_extract!(py_message, "type", String),
        "http.request".to_string()
      );
      assert_eq!(dict_extract!(py_message, "body", Vec<u8>), vec![1, 2, 3]);
      assert!(dict_extract!(py_message, "more_body", bool));
    });
  }

  #[test]
  fn test_http_send_message_from_pyobject() {
    Python::initialize();
    Python::attach(|py| {
      let dict = PyDict::new(py);
      dict.set_item("type", "http.response.start").unwrap();
      dict.set_item("status", 200).unwrap();

      // Headers should be a list of lists in ASGI format
      let headers = vec![vec!["content-type", "text/plain"]];
      dict.set_item("headers", headers).unwrap();
      dict.set_item("trailers", false).unwrap();

      let message: HttpSendMessage = dict.extract().unwrap();
      assert_eq!(
        message,
        HttpSendMessage::HttpResponseStart {
          status: 200,
          headers: vec![("content-type".to_string(), "text/plain".to_string())],
          trailers: false,
        }
      );
    });
  }
}
