use std::borrow::Cow;

use sa_ide::Analysis;
use sa_span::lsp::{from_lsp_position, to_lsp_range};
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink, Url,
};
use tracing::debug;

use crate::lsp_utils;

pub fn goto_definition(
    analysis: &Analysis,
    vfs: &VfsSnapshot,
    params: GotoDefinitionParams,
) -> Option<GotoDefinitionResponse> {
    let uri = &params.text_document_position_params.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "goto_definition: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "goto_definition: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "goto_definition: file text not found");
            return None;
        }
    };
    let position = params.text_document_position_params.position;
    let offset = match from_lsp_position(position, text) {
        Some(offset) => offset,
        None => {
            debug!(
                ?position,
                file_id = ?file_id,
                text_len = text.len(),
                "goto_definition: invalid position"
            );
            return None;
        }
    };
    let target = match analysis.goto_definition(file_id, offset) {
        Some(target) => target,
        None => {
            debug!(file_id = ?file_id, offset = ?offset, "goto_definition: no definition found");
            return None;
        }
    };

    let target_path = match vfs.path(target.file_id) {
        Some(path) => path.clone(),
        None => {
            debug!(
                target_file_id = ?target.file_id,
                "goto_definition: missing target path in VFS, falling back to DB"
            );
            analysis.file_path(target.file_id).as_ref().clone()
        }
    };
    let target_uri = match Url::from_file_path(target_path.as_str()) {
        Ok(uri) => uri,
        Err(()) => {
            debug!(
                target_file_id = ?target.file_id,
                target_path = %target_path,
                "goto_definition: failed to convert target path to URI"
            );
            return None;
        }
    };
    let target_text = match vfs.file_text(target.file_id) {
        Some(text) => Cow::Borrowed(text),
        None => {
            debug!(
                target_file_id = ?target.file_id,
                target_path = %target_path,
                "goto_definition: target text missing in VFS, falling back to DB"
            );
            Cow::Owned(analysis.file_text(target.file_id).to_string())
        }
    };
    let target_range = to_lsp_range(target.range, &target_text);

    if let Some(origin_range) = target.origin_range {
        let origin_range = to_lsp_range(origin_range, text);
        let link = LocationLink {
            origin_selection_range: Some(origin_range),
            target_uri,
            target_range,
            target_selection_range: target_range,
        };
        return Some(GotoDefinitionResponse::Link(vec![link]));
    }

    Some(Location::new(target_uri, target_range).into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sa_config::ResolvedFoundryConfig;
    use sa_ide::{AnalysisChange, AnalysisHost};
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace};
    use sa_span::{TextRange, TextSize, lsp::to_lsp_position, lsp::to_lsp_range};
    use sa_test_support::extract_offset;
    use sa_vfs::{Vfs, VfsChange};
    use tower_lsp::lsp_types::{
        GotoDefinitionParams, GotoDefinitionResponse, Location, TextDocumentIdentifier,
        TextDocumentPositionParams, Url,
    };

    use super::goto_definition;

    #[test]
    fn goto_definition_falls_back_to_db_path_when_vfs_missing() {
        let root = NormalizedPath::new("/workspace");
        let parent_path = NormalizedPath::new("/workspace/src/Parent.sol");
        let child_path = NormalizedPath::new("/workspace/src/Child.sol");

        let (parent_text, parent_offset) = extract_offset(
            r#"
contract Parent {
    function /*caret*/value() public pure returns (uint256) {
        return 1;
    }
}
"#,
        );
        let (child_text, child_offset) = extract_offset(
            r#"
import "./Parent.sol";

contract Child is Parent {
    function foo() public pure returns (uint256) {
        return val/*caret*/ue();
    }
}
"#,
        );

        let mut vfs = Vfs::default();
        vfs.apply_change(VfsChange::Set {
            path: parent_path.clone(),
            text: Arc::from(parent_text.clone()),
        });
        vfs.apply_change(VfsChange::Set {
            path: child_path.clone(),
            text: Arc::from(child_text.clone()),
        });
        let snapshot = vfs.snapshot();

        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root, profile.clone());
        let config = ResolvedFoundryConfig::new(workspace.clone(), profile);
        let mut host = AnalysisHost::new();
        let mut change = AnalysisChange::new();
        change.set_vfs(snapshot);
        change.set_config(config);
        host.apply_change(change);

        vfs.apply_change(VfsChange::Remove {
            path: parent_path.clone(),
        });
        let snapshot = vfs.snapshot();
        let mut change = AnalysisChange::new();
        change.set_vfs(snapshot.clone());
        host.apply_change(change);

        let analysis = host.snapshot();
        let child_uri = Url::from_file_path(child_path.as_str()).expect("child uri");
        let position = to_lsp_position(child_offset, &child_text);
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: child_uri },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response = goto_definition(&analysis, &snapshot, params);
        let location = match response {
            Some(GotoDefinitionResponse::Scalar(location)) => location,
            Some(GotoDefinitionResponse::Array(locations)) => {
                locations.into_iter().next().expect("location")
            }
            Some(GotoDefinitionResponse::Link(links)) => {
                let link = links.into_iter().next().expect("location link");
                Location::new(link.target_uri, link.target_range)
            }
            None => panic!("expected definition location"),
        };

        let expected_range = TextRange::at(parent_offset, TextSize::from(5));
        let expected_range = to_lsp_range(expected_range, &parent_text);
        let parent_uri = Url::from_file_path(parent_path.as_str()).expect("parent uri");
        assert_eq!(location.uri, parent_uri);
        assert_eq!(location.range, expected_range);
    }
}
