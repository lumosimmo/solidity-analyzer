use std::collections::HashMap;

use sa_sema::SemaSnapshot;
use sa_test_utils::FixtureBuilder;

#[test]
fn snapshot_maps_sources_and_linearized_bases() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Base.sol",
            r#"
contract Base {
    function foo() public pure returns (uint256) {
        return 1;
    }
}
"#,
        )
        .file(
            "src/Derived.sol",
            r#"
import "./Base.sol";

contract Derived is Base {
    function bar() public pure returns (uint256) {
        return foo();
    }
}
"#,
        )
        .build()
        .expect("fixture");

    let vfs = fixture.vfs_snapshot();
    let path_to_file_id = vfs
        .iter()
        .map(|(file_id, path)| (path.clone(), file_id))
        .collect::<HashMap<_, _>>();
    let snapshot = SemaSnapshot::new(fixture.config(), vfs, &path_to_file_id, None, true)
        .expect("sema snapshot");

    let base_file_id = fixture.file_id("src/Base.sol").expect("base file id");
    let derived_file_id = fixture.file_id("src/Derived.sol").expect("derived file id");

    let base_source = snapshot
        .source_id_for_file(base_file_id)
        .expect("base source id");
    let derived_source = snapshot
        .source_id_for_file(derived_file_id)
        .expect("derived source id");

    assert_eq!(snapshot.file_id_for_source(base_source), Some(base_file_id));
    assert_eq!(
        snapshot.file_id_for_source(derived_source),
        Some(derived_file_id)
    );

    snapshot.with_gcx(|gcx| {
        let derived_contract = gcx
            .hir
            .contract_ids()
            .find(|id| gcx.hir.contract(*id).name.to_string() == "Derived")
            .expect("Derived contract");
        let contract = gcx.hir.contract(derived_contract);
        assert!(
            contract.linearized_bases.len() > 1,
            "expected linearized bases for Derived"
        );
    });
}
