use std::path::{Path, PathBuf};

use sa_span::TextSize;
use sa_test_utils::FixtureBuilder;

#[test]
fn loads_multi_file_fixture() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(
            r#"
[profile.default]
remappings = ["lib/=lib/forge-std/"]
"#,
        )
        .file(
            "src/Main.sol",
            r#"
import "lib/Lib.sol";
contract Main { Lib lib; }
"#,
        )
        .file(
            "lib/forge-std/Lib.sol",
            r#"
contract Lib {}
"#,
        )
        .build()
        .expect("fixture");

    let main_id = fixture.file_id("src/Main.sol").expect("main file id");
    let lib_id = fixture
        .file_id("lib/forge-std/Lib.sol")
        .expect("lib file id");

    assert_ne!(main_id, lib_id);
    let vfs = fixture.vfs_snapshot();
    assert!(
        vfs.file_text(main_id)
            .expect("main text")
            .contains("contract Main")
    );
    assert!(
        vfs.file_text(lib_id)
            .expect("lib text")
            .contains("contract Lib")
    );

    let remappings = fixture.config().active_profile().remappings();
    let expected = normalize_path(&fixture.root().join("lib/forge-std"));
    assert!(remappings.iter().any(|remapping| {
        remapping.from() == "lib/" && normalize_path(Path::new(remapping.to())) == expected
    }));

    let analysis = fixture.analysis();
    let main_text = analysis.file_text(main_id);
    let offset = main_text.find("Lib lib").expect("Lib reference in main");
    let offset = TextSize::try_from(offset).expect("offset fits in TextSize");
    let target = analysis
        .goto_definition(main_id, offset)
        .expect("Lib definition");
    assert_eq!(target.file_id, lib_id);
}

#[test]
fn builds_analysis_snapshot_from_fixture() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
contract Main { uint256 value; }
"#,
        )
        .build()
        .expect("fixture");

    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");
    let text = analysis.file_text(file_id);

    assert!(text.contains("contract Main"));
    assert!(text.contains("uint256 value"));
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}
