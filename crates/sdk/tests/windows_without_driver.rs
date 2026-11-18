//! The real Windows backend on a machine without the driver installed: the one
//! part of it that can be checked without loading the driver. On a machine where
//! the driver *is* installed (the test VM) this test is expected to fail; run it
//! with `--skip` there.

#[cfg(windows)]
#[test]
fn without_the_driver_installed_create_reports_driver_not_installed() {
    let result = pattie::Device::builder().name("Pattie SDK test").create();

    assert_eq!(result.err(), Some(pattie::Error::DriverNotInstalled));
}
