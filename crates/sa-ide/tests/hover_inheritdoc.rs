use sa_paths::NormalizedPath;
use sa_span::TextSize;
use sa_test_support::{extract_offset, setup_analysis};
use sa_test_utils::FixtureBuilder;

fn hover_docs(text: &str, offset: TextSize) -> String {
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");
    let result = analysis.hover(file_id, offset).expect("hover result");
    result
        .contents
        .split_once("\n\n")
        .map(|(_, docs)| docs)
        .expect("docs")
        .to_string()
}

fn hover_docs_from_fixture(fixture: &sa_test_utils::Fixture, offset: TextSize) -> String {
    let analysis = fixture.analysis();
    let main_id = fixture.file_id("src/Main.sol").expect("main file id");
    let result = analysis.hover(main_id, offset).expect("hover result");
    result
        .contents
        .split_once("\n\n")
        .map(|(_, docs)| docs)
        .expect("docs")
        .to_string()
}

#[test]
fn hover_inheritdoc_merges_missing_tags() {
    let (text, offset) = extract_offset(
        r#"contract Base {
    /// @notice Base notice.
    /// @dev Base dev.
    /// @param amount Base amount.
    /// @return result Base result.
    /// @custom:since v1
    function foo(uint256 amount) public virtual returns (uint256 result) { return amount; }
}

contract Derived is Base {
    /// @notice Derived notice.
    /// @param amount Derived amount.
    /// @inheritdoc Base
    function foo(uint256 amount) public override returns (uint256 result) { return amount; }
}

contract Main {
    function run() public {
        Derived d = new Derived();
        d.fo/*caret*/o(1);
    }
}"#,
    );

    let docs = hover_docs(&text, offset);
    let expected = r#"**Notice**
Derived notice.

**Dev**
Base dev.

**Parameters**
- `amount`: Derived amount.

**Returns**
- `result`: Base result.

**Custom**
- `since`: v1"#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_keeps_param_names_from_base() {
    let (text, offset) = extract_offset(
        r#"contract Base {
    /// @param amount Base amount.
    function foo(uint256 amount) public virtual {}
}

contract Derived is Base {
    /// @inheritdoc Base
    function foo(uint256 value) public override {}
}

contract Main {
    function run() public {
        Derived d = new Derived();
        d.fo/*caret*/o(1);
    }
}"#,
    );

    let docs = hover_docs(&text, offset);
    let expected = r#"**Parameters**
- `amount`: Base amount."#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_matches_overload_by_signature() {
    let (text, offset) = extract_offset(
        r#"contract Base {
    /// @notice Uint variant.
    /// @param amount Uint amount.
    function foo(uint256 amount) public virtual {}

    /// @notice Address variant.
    /// @param who Address param.
    function foo(address who) public virtual {}
}

contract Derived is Base {
    /// @inheritdoc Base
    function foo(uint256 amount) public override {}
}

contract Main {
    function run() public {
        Derived d = new Derived();
        d.fo/*caret*/o(1);
    }
}"#,
    );

    let docs = hover_docs(&text, offset);
    let expected = r#"**Notice**
Uint variant.

**Parameters**
- `amount`: Uint amount."#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_supports_contract_level() {
    let (text, offset) = extract_offset(
        r#"/// @title Base title
/// @author Alice
/// @notice Base notice.
/// @dev Base dev.
/// @custom:security high
contract Base {}

/// @inheritdoc Base
contract Derived is Base {}

contract Main {
    Der/*caret*/ived d = Derived(address(0));
}"#,
    );

    let docs = hover_docs(&text, offset);
    let expected = r#"**Title**
Base title

**Author**
Alice

**Notice**
Base notice.

**Dev**
Base dev.

**Custom**
- `security`: high"#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_invalid_base_is_left_unresolved() {
    let (text, offset) = extract_offset(
        r#"contract Base {
    /// @notice Base notice.
    function foo(uint256 amount) public virtual {}
}

contract Derived is Base {
    /// @inheritdoc Missing
    function foo(uint256 amount) public override {}
}

contract Main {
    function run() public {
        Derived d = new Derived();
        d.fo/*caret*/o(1);
    }
}"#,
    );

    let docs = hover_docs(&text, offset);
    let expected = r#"**Inheritdoc**
- `Missing`"#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_resolves_across_files() {
    let (main_text, offset) = extract_offset(
        r#"import "./Derived.sol";

contract Main {
    function run() public {
        Derived d = new Derived();
        d.fo/*caret*/o(1);
    }
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Base.sol",
            r#"contract Base {
    /// @notice Base notice.
    /// @param amount Base amount.
    function foo(uint256 amount) public virtual {}
}"#,
        )
        .file(
            "src/Derived.sol",
            r#"import "./Base.sol";

contract Derived is Base {
    /// @inheritdoc Base
    function foo(uint256 amount) public override {}
}"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let docs = hover_docs_from_fixture(&fixture, offset);
    let expected = r#"**Notice**
Base notice.

**Parameters**
- `amount`: Base amount."#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_resolves_interface_across_files() {
    let (main_text, offset) = extract_offset(
        r#"import "./IRMLinearKink.sol";

contract Main {
    function run(IRMLinearKink irm) public view returns (uint256) {
        return irm.computeInterestR/*caret*/ate(address(0), 1, 2);
    }
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/IIRM.sol",
            r#"interface IIRM {
    /// @notice Perform potentially state mutating computation of the new interest rate.
    /// @param vault Address of the vault to compute the new interest rate for.
    /// @param cash Amount of assets held directly by the vault.
    /// @param borrows Amount of assets lent out to borrowers by the vault.
    /// @return rate The new interest rate in second percent yield (SPY), scaled by 1e27.
    function computeInterestRate(address vault, uint256 cash, uint256 borrows)
        external
        returns (uint256);
}"#,
        )
        .file(
            "src/IRMLinearKink.sol",
            r#"import "./IIRM.sol";

contract IRMLinearKink is IIRM {
    /// @inheritdoc IIRM
    function computeInterestRate(address vault, uint256 cash, uint256 borrows)
        external
        view
        override
        returns (uint256)
    {
        return 0;
    }
}"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let docs = hover_docs_from_fixture(&fixture, offset);
    let expected = r#"**Notice**
Perform potentially state mutating computation of the new interest rate.

**Parameters**
- `vault`: Address of the vault to compute the new interest rate for.
- `cash`: Amount of assets held directly by the vault.
- `borrows`: Amount of assets lent out to borrowers by the vault.

**Returns**
- `rate`: The new interest rate in second percent yield (SPY), scaled by 1e27."#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_contract_level_resolves_across_files() {
    let (main_text, offset) = extract_offset(
        r#"import "./Derived.sol";

contract Main {
    Der/*caret*/ived value = Derived(address(0));
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Base.sol",
            r#"/// @title Base title
/// @author Alice
/// @notice Base notice.
/// @dev Base dev.
/// @custom:security high
contract Base {}"#,
        )
        .file(
            "src/Derived.sol",
            r#"import "./Base.sol";

/// @inheritdoc Base
contract Derived is Base {}"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let docs = hover_docs_from_fixture(&fixture, offset);
    let expected = r#"**Title**
Base title

**Author**
Alice

**Notice**
Base notice.

**Dev**
Base dev.

**Custom**
- `security`: high"#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_contract_level_invalid_across_files() {
    let (main_text, offset) = extract_offset(
        r#"import "./Derived.sol";

contract Main {
    Der/*caret*/ived value = Derived(address(0));
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Base.sol", "contract Base {}")
        .file(
            "src/Derived.sol",
            r#"import "./Base.sol";

/// @inheritdoc Missing
contract Derived is Base {}"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let docs = hover_docs_from_fixture(&fixture, offset);
    let expected = r#"**Inheritdoc**
- `Missing`"#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_resolves_dependency_layouts() {
    let (main_text, offset) = extract_offset(
        r#"import "./Token.sol";

contract Main {
    function run(Token token) public {
        token.per/*caret*/mit(address(0), address(0));
    }
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(
            r#"
[profile.default]
remappings = ["@openzeppelin/=lib/openzeppelin-contracts/"]
"#,
        )
        .file(
            "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Permit.sol",
            r#"interface IERC20Permit {
    /// @notice Approve spending by signature.
    /// @param owner The owner.
    /// @param spender The spender.
    function permit(address owner, address spender) external;
}"#,
        )
        .file(
            "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/ERC20Permit.sol",
            r#"import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";

contract ERC20Permit is IERC20Permit {
    /// @inheritdoc IERC20Permit
    function permit(address owner, address spender) external override {}
}"#,
        )
        .file(
            "src/Token.sol",
            r#"import "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";

contract Token is ERC20Permit {}"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let docs = hover_docs_from_fixture(&fixture, offset);
    let expected = r#"**Notice**
Approve spending by signature.

**Parameters**
- `owner`: The owner.
- `spender`: The spender."#;
    assert_eq!(docs, expected);
}

#[test]
fn hover_inheritdoc_resolves_dependency_files_directly() {
    let (lib_text, offset) = extract_offset(
        r#"import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";

contract ERC20Permit is IERC20Permit {
    /// @inheritdoc IERC20Permit
    function per/*caret*/mit(address owner, address spender) external override {}
}"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(
            r#"
[profile.default]
remappings = ["@openzeppelin/=lib/openzeppelin-contracts/"]
"#,
        )
        .file(
            "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Permit.sol",
            r#"interface IERC20Permit {
    /// @notice Approve spending by signature.
    /// @param owner The owner.
    /// @param spender The spender.
    function permit(address owner, address spender) external;
}"#,
        )
        .file(
            "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/ERC20Permit.sol",
            lib_text,
        )
        .build()
        .expect("fixture");

    let analysis = fixture.analysis();
    let lib_id = fixture
        .file_id("lib/openzeppelin-contracts/contracts/token/ERC20/extensions/ERC20Permit.sol")
        .expect("lib file id");
    let result = analysis.hover(lib_id, offset).expect("hover result");
    let docs = result
        .contents
        .split_once("\n\n")
        .map(|(_, docs)| docs)
        .expect("docs")
        .to_string();
    let expected = r#"**Notice**
Approve spending by signature.

**Parameters**
- `owner`: The owner.
- `spender`: The spender."#;
    assert_eq!(docs, expected);
}
