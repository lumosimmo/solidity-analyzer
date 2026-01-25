use sa_ide::Analysis;
use sa_span::lsp::{from_lsp_position, to_lsp_range};
use sa_span::{TextRange, TextSize};
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind};
use tracing::debug;

use crate::lsp_utils;

pub fn hover(analysis: &Analysis, vfs: &VfsSnapshot, params: HoverParams) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "hover: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "hover: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "hover: file text not found");
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
                "hover: invalid position"
            );
            return None;
        }
    };
    let hover = match analysis.hover(file_id, offset) {
        Some(hover) => hover,
        None => {
            debug!(file_id = ?file_id, offset = ?offset, "hover: no result");
            return None;
        }
    };

    let range = hover_range_in_bounds(hover.range, text).map(|range| to_lsp_range(range, text));
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: hover.contents,
        }),
        range,
    })
}

fn hover_range_in_bounds(range: TextRange, text: &str) -> Option<TextRange> {
    let text_len = TextSize::of(text);
    if range.end() <= text_len {
        Some(range)
    } else {
        debug!(?range, ?text_len, "hover: range out of bounds");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::hover_range_in_bounds;
    use sa_span::{TextRange, TextSize};

    #[test]
    fn hover_range_out_of_bounds_is_dropped() {
        let text = "contract Foo {}";
        let range = TextRange::new(TextSize::from(0), TextSize::from(128));
        assert!(hover_range_in_bounds(range, text).is_none());
    }

    #[test]
    fn hover_range_in_bounds_is_kept() {
        let text = "contract Foo {}";
        let range = TextRange::new(TextSize::from(0), TextSize::from(8));
        assert_eq!(hover_range_in_bounds(range, text), Some(range));
    }
}
