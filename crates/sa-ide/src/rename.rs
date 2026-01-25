use sa_base_db::{FileId, ProjectId};
use sa_hir::{Definition, Semantics};
use sa_span::{TextSize, is_ident_byte};

use crate::{Reference, SourceChange, TextEdit};

pub fn rename(
    db: &dyn sa_ide_db::IdeDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
    new_name: &str,
) -> Option<SourceChange> {
    if !is_valid_identifier(new_name) {
        return None;
    }

    let semantics = Semantics::new(db, project_id);
    let definition = semantics.resolve_definition(file_id, offset)?;
    let refs = match definition {
        Definition::Global(def_id) => sa_ide_db::find_references(db, project_id, def_id),
        Definition::Local(local) => sa_hir::local_references(db, file_id, &local)
            .into_iter()
            .map(|range| Reference::new(file_id, range))
            .collect(),
    };
    build_source_change(refs, new_name)
}

fn build_source_change(references: Vec<Reference>, new_name: &str) -> Option<SourceChange> {
    if references.is_empty() {
        return None;
    }

    let mut change = SourceChange::default();
    for reference in references {
        change.insert_edit(
            reference.file_id(),
            TextEdit {
                range: reference.range(),
                new_text: new_name.to_string(),
            },
        );
    }
    change.normalize();
    Some(change)
}

fn is_valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !is_ident_byte(first) || first.is_ascii_digit() {
        return false;
    }
    bytes.all(is_ident_byte)
}
