use crate::{XtaskError, workspace_root};
use std::fs;

const GENERATED_CONTENT: &str = "solidity-analyzer codegen fixture\n";
const FIXTURE_PATH: &str = "xtask/fixtures/codegen-output.txt";

pub(crate) fn run(check: bool) -> Result<(), XtaskError> {
    let workspace_root = workspace_root()?;
    let fixture_path = workspace_root.join(FIXTURE_PATH);

    if check {
        let current = fs::read_to_string(&fixture_path).map_err(|err| {
            XtaskError::new(format!("failed to read fixture {fixture_path:?}: {err}"))
        })?;
        if current != GENERATED_CONTENT {
            return Err(XtaskError::new(
                "codegen output is out of date; run `cargo run -p xtask -- codegen`",
            ));
        }
        Ok(())
    } else {
        if let Some(parent) = fixture_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                XtaskError::new(format!("failed to create fixture dir {parent:?}: {err}"))
            })?;
        }
        fs::write(&fixture_path, GENERATED_CONTENT).map_err(|err| {
            XtaskError::new(format!("failed to write fixture {fixture_path:?}: {err}"))
        })?;
        Ok(())
    }
}
