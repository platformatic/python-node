use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// ASGI information containing ASGI version and protocol spec version
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsgiInfo {
  pub version: String,
  pub spec_version: String,
}

impl AsgiInfo {
  pub fn new<A, B>(version: A, spec_version: B) -> Self
  where
    A: Into<String>,
    B: Into<String>,
  {
    AsgiInfo {
      version: version.into(),
      spec_version: spec_version.into(),
    }
  }
}

impl<'py> IntoPyObject<'py> for AsgiInfo {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
    let dict = PyDict::new(py);
    dict.set_item("version", self.version)?;
    dict.set_item("spec_version", self.spec_version)?;
    Ok(dict)
  }
}

impl<'py> FromPyObject<'py> for AsgiInfo {
  fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
    let dict = ob.downcast::<PyDict>()?;
    let version: String = dict
      .get_item("version")?
      .ok_or_else(|| PyValueError::new_err("Missing 'version' key in ASGI info dictionary"))?
      .extract()?;
    let spec_version: String = dict
      .get_item("spec_version")?
      .ok_or_else(|| PyValueError::new_err("Missing 'spec_version' key in ASGI info dictionary"))?
      .extract()?;
    Ok(AsgiInfo::new(version, spec_version))
  }
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
  fn test_asgi_info_pyobject_conversion() {
    Python::with_gil(|py| {
      let asgi_info = AsgiInfo::new("3.0", "2.5");

      // Convert AsgiInfo to PyObject
      let dict = asgi_info.clone().into_pyobject(py).unwrap();
      assert_eq!(dict_extract!(dict, "version", String), "3.0");
      assert_eq!(dict_extract!(dict, "spec_version", String), "2.5");

      // Convert back to AsgiInfo
      let extracted: AsgiInfo = dict.extract().unwrap();
      assert_eq!(extracted, asgi_info);
    });
  }
}
