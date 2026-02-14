use std::path::Path;

#[test]
fn test_loader() {
    let loader_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-conformance/loader");
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");

    let result =
        duralade_conformance_harness::loader::run_loader_tests(&loader_dir, &stdlib_dir).unwrap();

    if !result.passed {
        panic!("Loader tests failed:\n{}", result.diagnostics.join("\n"));
    }
}
