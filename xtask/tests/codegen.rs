#[test]
fn codegen_check_matches_fixture() {
    xtask::run(["codegen", "--check"]).expect("codegen check should pass");
}
