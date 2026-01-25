use sa_def::DefKind;
use sa_paths::NormalizedPath;
use sa_test_support::setup_analysis;

#[test]
fn workspace_symbols_searches_across_files() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            "contract Main {}".to_string(),
        ),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Lib.sol"))
        .expect("lib file id");
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    // Test searching for Lib
    let symbols = analysis.workspace_symbols("Lib");
    assert_eq!(symbols.len(), 1);
    let symbol = &symbols[0];
    assert_eq!(symbol.name(), "Lib");
    assert_eq!(symbol.file_id(), lib_id);
    assert_eq!(symbol.kind(), DefKind::Contract);

    // Test searching for Main
    let main_symbols = analysis.workspace_symbols("Main");
    assert_eq!(main_symbols.len(), 1);
    let main_symbol = &main_symbols[0];
    assert_eq!(main_symbol.name(), "Main");
    assert_eq!(main_symbol.file_id(), main_id);
    assert_eq!(main_symbol.kind(), DefKind::Contract);

    // Negative case: searching for non-existent symbol yields empty result
    let not_found = analysis.workspace_symbols("DoesNotExist");
    assert!(not_found.is_empty());
}
