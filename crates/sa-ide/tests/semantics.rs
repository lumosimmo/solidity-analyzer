use std::sync::Arc;

use sa_ide::{Analysis, AnalysisChange, AnalysisHost};
use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offset, extract_offsets, find_range};
use sa_test_utils::FixtureBuilder;
use sa_vfs::{FileId, Vfs, VfsChange};

fn analysis_from_vfs(path: &str, text: &str) -> (Analysis, FileId) {
    let mut vfs = Vfs::default();
    let path = NormalizedPath::new(path);
    vfs.apply_change(VfsChange::Set {
        path: path.clone(),
        text: Arc::from(text),
    });
    let snapshot = vfs.snapshot();
    let file_id = snapshot.file_id(&path).expect("file id");

    let mut host = AnalysisHost::new();
    let mut change = AnalysisChange::new();
    change.set_vfs(snapshot);
    host.apply_change(change);

    (host.snapshot(), file_id)
}

struct ReexportFixture {
    analysis: Analysis,
    main_id: FileId,
    base_id: FileId,
    offset: TextSize,
}

fn reexport_fixture(use_alias: bool) -> ReexportFixture {
    let base_text = r#"
contract Base {}
"#;

    let intermediate_text = if use_alias {
        r#"
import {Base as AliasBase} from "./Base.sol";

contract Intermediate is AliasBase {}
"#
    } else {
        r#"
import {Base} from "./Base.sol";

contract Intermediate is Base {}
"#
    };

    let (main_text, offset) = if use_alias {
        extract_offset(
            r#"
import {Intermediate, AliasBase} from "./Intermediate.sol";

contract Main is Intermediate {
    Ali/*caret*/asBase value;
}
"#,
        )
    } else {
        extract_offset(
            r#"
import {Intermediate, Base} from "./Intermediate.sol";

contract Main is Intermediate {
    Ba/*caret*/se value;
}
"#,
        )
    };

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Base.sol", base_text)
        .file("src/Intermediate.sol", intermediate_text)
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let main_id = fixture.file_id("src/Main.sol").expect("main file id");
    let base_id = fixture.file_id("src/Base.sol").expect("base file id");

    ReexportFixture {
        analysis,
        main_id,
        base_id,
        offset,
    }
}

#[test]
fn goto_definition_resolves_across_files() {
    let (main_text, offset) = extract_offset(
        r#"
import "lib/Lib.sol";
contract Main { Li/*caret*/b lib; }
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(
            r#"
[profile.default]
remappings = ["lib/=lib/forge-std/"]
"#,
        )
        .file("src/Main.sol", main_text)
        .file(
            "lib/forge-std/Lib.sol",
            r#"
contract Lib {}
"#,
        )
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let main_id = fixture.file_id("src/Main.sol").expect("main file id");
    let lib_id = fixture
        .file_id("lib/forge-std/Lib.sol")
        .expect("lib file id");

    let target = analysis
        .goto_definition(main_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, lib_id);

    let lib_text = analysis.file_text(lib_id);
    let name_start = lib_text.find("Lib").expect("Lib definition");
    let expected = sa_span::TextRange::new(
        TextSize::from(name_start as u32),
        TextSize::from((name_start + 3) as u32),
    );
    assert_eq!(target.range, expected);
}

#[test]
fn find_references_returns_sorted_results() {
    let (main_text, offset) = extract_offset(
        r#"
import "lib/Lib.sol";
contract Main { Li/*caret*/b lib; }
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(
            r#"
[profile.default]
remappings = ["lib/=lib/forge-std/"]
"#,
        )
        .file("src/Main.sol", main_text)
        .file("lib/forge-std/Lib.sol", "contract Lib {}")
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let main_id = fixture.file_id("src/Main.sol").expect("main file id");

    let refs = analysis.find_references(main_id, offset);
    assert!(!refs.is_empty());
    let mut sorted = refs.clone();
    sorted.sort_by_key(|reference| (reference.file_id(), reference.range().start()));
    assert_eq!(refs, sorted);
}

#[test]
fn goto_definition_resolves_local_parameter() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        val/*caret*/ue;
    }
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = find_range(&text, "value");
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_parameter_definition_site() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 val/*caret*/ue) public {
        value;
    }
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = find_range(&text, "value");
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_handles_missing_workspace() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        val/*caret*/ue;
    }
}
"#,
    );
    let (analysis, file_id) = analysis_from_vfs("/workspace/src/Main.sol", &text);
    let target = analysis
        .goto_definition(file_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = find_range(&text, "value");
    assert_eq!(target.range, expected);
}

#[test]
fn find_references_without_workspace_returns_empty() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo() public {}
    function bar() public {
        fo/*caret*/o();
    }
}
"#,
    );
    let (analysis, file_id) = analysis_from_vfs("/workspace/src/Main.sol", &text);

    let refs = analysis.find_references(file_id, offset);
    assert!(refs.is_empty());
}

#[test]
fn goto_definition_import_path_without_workspace_returns_none() {
    let (text, offset) = extract_offset(
        r#"
import "lib/Lib./*caret*/sol";
contract Main {}
"#,
    );
    let (analysis, file_id) = analysis_from_vfs("/workspace/src/Main.sol", &text);

    let target = analysis.goto_definition(file_id, offset);
    assert!(target.is_none());
}

#[test]
fn goto_definition_returns_none_for_ambiguous_import_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import { Lib as Alias } from "./LibA.sol";
import { Lib as Alias } from "./LibB.sol";

contract Main {
    Ali/*caret*/as value;
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .file(
            "src/LibA.sol",
            r#"
contract Lib {}
"#,
        )
        .file(
            "src/LibB.sol",
            r#"
contract Lib {}
"#,
        )
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let main_id = fixture.file_id("src/Main.sol").expect("main file id");

    let target = analysis.goto_definition(main_id, offset);
    assert!(target.is_none());
}

#[test]
fn goto_definition_resolves_local_variable() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo() public {
        uint256 amount = 1;
        return am/*caret*/ount;
    }
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = find_range(&text, "amount");
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_inherited_function_in_diamond() {
    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function /*def*/foo() public virtual override {}
}

contract C is A {}

contract D is B, C {
    function bar() public {
        fo/*caret*/o();
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let call_offset = offsets[1];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, call_offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = TextRange::at(def_offset, TextSize::from(3));
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_super_call() {
    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function /*def*/foo() public virtual override {}
}

contract C is A {
    function foo() public virtual override {}
}

contract D is B, C {
    function foo() public override(B, C) {
        super.fo/*caret*/o();
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let call_offset = offsets[1];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, call_offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = TextRange::at(def_offset, TextSize::from(3));
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_named_return() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo() public returns (uint256 total) {
        tot/*caret*/al = 1;
        return total;
    }
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text.clone())
        .build()
        .expect("fixture");
    let analysis = fixture.analysis();
    let file_id = fixture.file_id("src/Main.sol").expect("file id");

    let target = analysis
        .goto_definition(file_id, offset)
        .expect("definition");
    assert_eq!(target.file_id, file_id);
    let expected = find_range(&text, "total");
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_reexported_import() {
    let fixture = reexport_fixture(false);
    let target = fixture
        .analysis
        .goto_definition(fixture.main_id, fixture.offset)
        .expect("definition");
    assert_eq!(target.file_id, fixture.base_id);

    let base_text = fixture.analysis.file_text(fixture.base_id);
    let name_start = base_text.find("Base").expect("Base definition");
    let expected = TextRange::new(
        TextSize::from(name_start as u32),
        TextSize::from((name_start + 4) as u32),
    );
    assert_eq!(target.range, expected);
}

#[test]
fn goto_definition_resolves_reexported_alias() {
    let fixture = reexport_fixture(true);
    let target = fixture
        .analysis
        .goto_definition(fixture.main_id, fixture.offset)
        .expect("definition");
    assert_eq!(target.file_id, fixture.base_id);

    let base_text = fixture.analysis.file_text(fixture.base_id);
    let name_start = base_text.find("Base").expect("Base definition");
    let expected = TextRange::new(
        TextSize::from(name_start as u32),
        TextSize::from((name_start + 4) as u32),
    );
    assert_eq!(target.range, expected);
}
