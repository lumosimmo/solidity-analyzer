use std::path::Path;

#[test]
fn build_scripts_exist() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("xtask should live under the workspace root");

    let scripts = [
        "scripts/build-linux.sh",
        "scripts/build-macos.sh",
        "scripts/build-windows.ps1",
        "scripts/test-linux.sh",
        "scripts/test-macos.sh",
        "scripts/test-windows.ps1",
    ];

    for script in scripts {
        let path = workspace_root.join(script);
        assert!(path.exists(), "missing script: {script}");
    }
}
