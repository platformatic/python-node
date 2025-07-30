use std::convert::Infallible;
use std::str::FromStr;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;

/// HTTP version of an ASGI server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HttpVersion {
  V1_0,
  #[default]
  V1_1,
  V2_0,
}

impl<'py> FromPyObject<'py> for HttpVersion {
  fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
    let version: String = ob.extract()?;
    match version.as_str() {
      "1" | "1.0" => Ok(HttpVersion::V1_0),
      "1.1" => Ok(HttpVersion::V1_1),
      "2" | "2.0" => Ok(HttpVersion::V2_0),
      _ => Err(PyValueError::new_err(format!(
        "Invalid HTTP version: {version}"
      ))),
    }
  }
}

impl<'py> IntoPyObject<'py> for HttpVersion {
  type Target = PyString;
  type Output = Bound<'py, Self::Target>;
  type Error = Infallible;

  fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
    match self {
      HttpVersion::V1_0 => "1.0".into_pyobject(py),
      HttpVersion::V1_1 => "1.1".into_pyobject(py),
      HttpVersion::V2_0 => "2.0".into_pyobject(py),
    }
  }
}

impl FromStr for HttpVersion {
  type Err = String;

  fn from_str(version: &str) -> Result<HttpVersion, Self::Err> {
    match version {
      "1" | "1.0" => Ok(HttpVersion::V1_0),
      "1.1" => Ok(HttpVersion::V1_1),
      "2" | "2.0" => Ok(HttpVersion::V2_0),
      _ => Err(format!("Invalid HTTP version: {version}")),
    }
  }
}

impl From<HttpVersion> for String {
  fn from(version: HttpVersion) -> String {
    match version {
      HttpVersion::V1_0 => "1.0".to_string(),
      HttpVersion::V1_1 => "1.1".to_string(),
      HttpVersion::V2_0 => "2.0".to_string(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_http_version_pyobject_conversion() {
    Python::with_gil(|py| {
      let tests = vec![
        (HttpVersion::V1_0, "1.0"),
        (HttpVersion::V1_1, "1.1"),
        (HttpVersion::V2_0, "2.0"),
      ];

      for (http_version, version_str) in tests {
        // Convert HttpVersion to PyObject
        let py_value = http_version.into_pyobject(py).unwrap();
        assert_eq!(py_value, version_str);

        // Convert back to HttpVersion
        let extracted: HttpVersion = py_value.extract().unwrap();
        assert_eq!(extracted, http_version);
      }

      let shorthands = vec![(HttpVersion::V1_0, "1"), (HttpVersion::V2_0, "2")];

      for (http_version, shorthand) in shorthands {
        // Convert shorthand to HttpVersion
        let py_value = shorthand.into_pyobject(py).unwrap();
        let extracted: HttpVersion = py_value.extract().unwrap();
        assert_eq!(extracted, http_version);
      }
    });
  }

  #[test]
  fn test_http_version_string_conversion() {
    let tests = vec![
      ("1.0", HttpVersion::V1_0),
      ("1.1", HttpVersion::V1_1),
      ("2.0", HttpVersion::V2_0),
    ];

    for (version_str, expected) in tests {
      let version = HttpVersion::from_str(version_str).unwrap();
      assert_eq!(version, expected);
      assert_eq!(String::from(version), version_str.to_string());
    }

    let shorthands = vec![("1", HttpVersion::V1_0), ("2", HttpVersion::V2_0)];

    for (shorthand, expected) in shorthands {
      let version = HttpVersion::from_str(shorthand).unwrap();
      assert_eq!(version, expected);
      assert_eq!(String::from(version), format!("{shorthand}.0"));
    }

    // Test invalid version
    let invalid_version = "3.0";
    assert!(HttpVersion::from_str(invalid_version).is_err());
  }
}
