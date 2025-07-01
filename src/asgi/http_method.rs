use std::convert::Infallible;
use std::str::FromStr;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;

/// HTTP methods used in an ASGI server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HttpMethod {
  #[default]
  Get,
  Post,
  Put,
  Delete,
  Patch,
  Head,
  Options,
  Trace,
  Connect,
}

impl<'py> FromPyObject<'py> for HttpMethod {
  fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
    let method: String = ob.extract()?;
    method
      .to_uppercase()
      .as_str()
      .parse()
      .map_err(PyValueError::new_err)
  }
}

impl<'py> IntoPyObject<'py> for HttpMethod {
  type Target = PyString;
  type Output = Bound<'py, Self::Target>;
  type Error = Infallible;

  fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
    match self {
      HttpMethod::Get => "GET".into_pyobject(py),
      HttpMethod::Post => "POST".into_pyobject(py),
      HttpMethod::Put => "PUT".into_pyobject(py),
      HttpMethod::Delete => "DELETE".into_pyobject(py),
      HttpMethod::Patch => "PATCH".into_pyobject(py),
      HttpMethod::Head => "HEAD".into_pyobject(py),
      HttpMethod::Options => "OPTIONS".into_pyobject(py),
      HttpMethod::Trace => "TRACE".into_pyobject(py),
      HttpMethod::Connect => "CONNECT".into_pyobject(py),
    }
  }
}

impl FromStr for HttpMethod {
  type Err = String;

  fn from_str(method: &str) -> Result<HttpMethod, Self::Err> {
    match method.to_uppercase().as_str() {
      "GET" => Ok(HttpMethod::Get),
      "POST" => Ok(HttpMethod::Post),
      "PUT" => Ok(HttpMethod::Put),
      "DELETE" => Ok(HttpMethod::Delete),
      "PATCH" => Ok(HttpMethod::Patch),
      "HEAD" => Ok(HttpMethod::Head),
      "OPTIONS" => Ok(HttpMethod::Options),
      "TRACE" => Ok(HttpMethod::Trace),
      "CONNECT" => Ok(HttpMethod::Connect),
      _ => Err(format!("Invalid HTTP method: {method}")),
    }
  }
}

impl TryFrom<String> for HttpMethod {
  type Error = String;

  fn try_from(method: String) -> Result<HttpMethod, Self::Error> {
    method.as_str().parse()
  }
}

impl TryFrom<&http_handler::Method> for HttpMethod {
  type Error = String;

  fn try_from(method: &http_handler::Method) -> Result<HttpMethod, Self::Error> {
    match *method {
      http_handler::Method::GET => Ok(HttpMethod::Get),
      http_handler::Method::POST => Ok(HttpMethod::Post),
      http_handler::Method::PUT => Ok(HttpMethod::Put),
      http_handler::Method::DELETE => Ok(HttpMethod::Delete),
      http_handler::Method::PATCH => Ok(HttpMethod::Patch),
      http_handler::Method::HEAD => Ok(HttpMethod::Head),
      http_handler::Method::OPTIONS => Ok(HttpMethod::Options),
      http_handler::Method::TRACE => Ok(HttpMethod::Trace),
      http_handler::Method::CONNECT => Ok(HttpMethod::Connect),
      _ => Err(format!("Invalid HTTP method: {method}")),
    }
  }
}

impl From<HttpMethod> for String {
  fn from(method: HttpMethod) -> String {
    match method {
      HttpMethod::Get => "GET".to_string(),
      HttpMethod::Post => "POST".to_string(),
      HttpMethod::Put => "PUT".to_string(),
      HttpMethod::Delete => "DELETE".to_string(),
      HttpMethod::Patch => "PATCH".to_string(),
      HttpMethod::Head => "HEAD".to_string(),
      HttpMethod::Options => "OPTIONS".to_string(),
      HttpMethod::Trace => "TRACE".to_string(),
      HttpMethod::Connect => "CONNECT".to_string(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_http_method_pyobject_conversion() {
    Python::with_gil(|py| {
      let tests = vec![
        (HttpMethod::Get, "GET"),
        (HttpMethod::Post, "POST"),
        (HttpMethod::Put, "PUT"),
        (HttpMethod::Delete, "DELETE"),
        (HttpMethod::Patch, "PATCH"),
        (HttpMethod::Options, "OPTIONS"),
        (HttpMethod::Head, "HEAD"),
      ];

      for (http_method, method_str) in tests {
        // Convert HttpMethods to PyObject
        let py_value = http_method.into_pyobject(py).unwrap();
        assert_eq!(py_value, method_str);

        // Convert back to HttpMethods
        let extracted: HttpMethod = py_value.extract().unwrap();
        assert_eq!(extracted, http_method);
      }
    });
  }

  #[test]
  fn test_http_method_string_conversion() {
    let methods = vec![
      ("GET", HttpMethod::Get),
      ("POST", HttpMethod::Post),
      ("PUT", HttpMethod::Put),
      ("DELETE", HttpMethod::Delete),
      ("PATCH", HttpMethod::Patch),
      ("HEAD", HttpMethod::Head),
      ("OPTIONS", HttpMethod::Options),
      ("TRACE", HttpMethod::Trace),
      ("CONNECT", HttpMethod::Connect),
    ];

    for (method_str, http_method) in methods {
      let parsed: HttpMethod = method_str.parse().expect("should parse valid HTTP method");

      assert_eq!(parsed, http_method);
      assert_eq!(String::from(http_method), method_str);
    }

    // Test invalid method
    assert!(HttpMethod::try_from("INVALID".to_string()).is_err());
  }

  #[test]
  fn test_http_method_try_from_http_handler_method() {
    let test_cases = vec![
      (http_handler::Method::GET, HttpMethod::Get),
      (http_handler::Method::POST, HttpMethod::Post),
      (http_handler::Method::PUT, HttpMethod::Put),
      (http_handler::Method::DELETE, HttpMethod::Delete),
      (http_handler::Method::PATCH, HttpMethod::Patch),
      (http_handler::Method::HEAD, HttpMethod::Head),
      (http_handler::Method::OPTIONS, HttpMethod::Options),
      (http_handler::Method::TRACE, HttpMethod::Trace),
      (http_handler::Method::CONNECT, HttpMethod::Connect),
    ];

    for (http_handler_method, expected_asgi_method) in test_cases {
      let result: Result<HttpMethod, String> = (&http_handler_method).try_into();
      assert!(result.is_ok(), "Failed to convert {http_handler_method}");
      assert_eq!(result.unwrap(), expected_asgi_method);
    }
  }
}
