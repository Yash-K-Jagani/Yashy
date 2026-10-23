#[test]
fn detect_reports_missing_toolchain_when_nothing_is_installed() {
    // Force the failure path by pointing every probe at a nonexistent binary.
    let err = yashy_driver::toolchain::detect_with(|name| {
        if name == "llc" { Some(std::path::PathBuf::from("/nonexistent/llc")) } else { None }
    });
    assert!(err.is_err());
    let msg = err.unwrap_err();
    assert!(msg.contains("llc"), "error must name the binary it looked for: {msg}");
}
