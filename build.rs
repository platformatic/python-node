#[cfg(feature = "napi-support")]
extern crate napi_build;

fn main() {
  #[cfg(feature = "napi-support")]
  napi_build::setup();
}
