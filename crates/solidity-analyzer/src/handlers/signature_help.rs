use sa_ide::{ParameterInformation, SignatureHelp, SignatureInformation};
use sa_span::lsp::from_lsp_position;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterLabel, SignatureHelp as LspSignatureHelp,
    SignatureHelpParams, SignatureInformation as LspSignatureInformation,
};
use tracing::debug;

use crate::lsp_utils;

pub fn signature_help(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: SignatureHelpParams,
) -> Option<LspSignatureHelp> {
    let uri = &params.text_document_position_params.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "signature_help: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "signature_help: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "signature_help: file text not found");
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
                "signature_help: invalid position"
            );
            return None;
        }
    };
    let help = match analysis.signature_help(file_id, offset) {
        Some(help) => help,
        None => {
            debug!(file_id = ?file_id, offset = ?offset, "signature_help: no result");
            return None;
        }
    };

    Some(signature_help_to_lsp(help))
}

fn signature_help_to_lsp(help: SignatureHelp) -> LspSignatureHelp {
    let signatures = help
        .signatures
        .into_iter()
        .map(signature_to_lsp)
        .collect::<Vec<_>>();
    LspSignatureHelp {
        signatures,
        active_signature: help.active_signature.map(|idx| idx as u32),
        active_parameter: help.active_parameter.map(|idx| idx as u32),
    }
}

fn signature_to_lsp(signature: SignatureInformation) -> LspSignatureInformation {
    let documentation = signature.documentation.map(|doc| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc,
        })
    });
    let parameters = signature
        .parameters
        .into_iter()
        .map(parameter_to_lsp)
        .collect::<Vec<_>>();

    LspSignatureInformation {
        label: signature.label,
        documentation,
        parameters: Some(parameters),
        active_parameter: None,
    }
}

fn parameter_to_lsp(parameter: ParameterInformation) -> tower_lsp::lsp_types::ParameterInformation {
    tower_lsp::lsp_types::ParameterInformation {
        label: ParameterLabel::Simple(parameter.label),
        documentation: None,
    }
}
