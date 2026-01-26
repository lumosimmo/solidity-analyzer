use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sa_base_db::{FileId, ProjectId};
use sa_def::{DefEntry, DefKind};
use sa_hir::{HirDatabase, lowered_program};
use sa_sema::{SemaFunctionSignature, sema_snapshot_for_project};
use sa_span::TextRange;
use sa_syntax::{
    Parse,
    ast::{
        CommentKind, DocComment, Item, ItemFunction, ItemKind, NatSpecItem, NatSpecKind, Type,
        VariableDefinition,
    },
};
use tracing::debug;
use url::Url;

pub fn find_item_by_name_range<'a>(
    parse: &'a Parse,
    container: Option<&str>,
    name_range: TextRange,
) -> Option<&'a Item<'static>> {
    parse.with_session(|| {
        let mut items = if let Some(container_name) = container {
            let contract = parse
                .tree()
                .items
                .iter()
                .find_map(|item| match &item.kind {
                    ItemKind::Contract(contract) if contract.name.to_string() == container_name => {
                        Some(contract)
                    }
                    _ => None,
                })?;
            contract.body.iter()
        } else {
            parse.tree().items.iter()
        };

        items.find(|item| {
            let Some(name) = item.name() else {
                return false;
            };
            parse.span_to_text_range(name.span) == Some(name_range)
        })
    })
}

/// Formats a function signature for display in hover or signature help.
pub fn format_function_signature(parse: &Parse, text: &str, function: &ItemFunction<'_>) -> String {
    let kind = function.kind.to_str();
    let name = parse.with_session(|| function.header.name.map(|ident| ident.to_string()));
    let mut signature = String::new();
    signature.push_str(kind);
    if let Some(name) = name {
        signature.push(' ');
        signature.push_str(&name);
    }
    signature.push('(');
    let params = function
        .header
        .parameters
        .vars
        .iter()
        .map(|param| format_param(parse, text, param))
        .collect::<Vec<_>>();
    signature.push_str(&params.join(", "));
    signature.push(')');

    let returns = function
        .header
        .returns
        .as_ref()
        .map(|returns| {
            returns
                .vars
                .iter()
                .map(|param| format_param(parse, text, param))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !returns.is_empty() {
        signature.push_str(" returns (");
        signature.push_str(&returns.join(", "));
        signature.push(')');
    }

    signature
}

/// Formats a parameter (variable definition) for display.
pub fn format_param(parse: &Parse, text: &str, param: &VariableDefinition<'_>) -> String {
    let ty = type_text(parse, text, &param.ty).unwrap_or_else(|| "unknown".to_string());
    let name = parse.with_session(|| param.name.map(|ident| ident.to_string()));
    match name {
        Some(name) => format!("{ty} {name}"),
        None => ty,
    }
}

/// Extracts the text representation of a type from source code.
pub fn type_text(parse: &Parse, text: &str, ty: &Type<'_>) -> Option<String> {
    let range = parse.span_to_text_range(ty.span)?;
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    text.get(start..end).map(|slice| slice.trim().to_string())
}

pub fn sema_function_signature_for_entry(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    entry: &DefEntry,
) -> Option<SemaFunctionSignature> {
    let project = db.project_input(project_id);
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(entry.location().file_id())?;
    snapshot.function_signature_for_definition(
        entry.location().file_id(),
        entry.location().range(),
        entry.location().name(),
        entry.container(),
    )
}

pub fn sema_variable_label_for_entry(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    entry: &DefEntry,
) -> Option<String> {
    let project = db.project_input(project_id);
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(entry.location().file_id())?;
    snapshot.variable_label_for_definition(
        entry.location().file_id(),
        entry.location().range(),
        entry.location().name(),
        entry.container(),
    )
}

/// Extracts documentation comments from an AST item.
///
/// Collects all doc comments from the item, trims whitespace, filters empty
/// entries, and joins them with newlines.
///
/// # Returns
/// The combined documentation string, or `None` if no documentation exists.
pub fn docs_for_item(parse: &Parse, item: &Item<'static>) -> Option<String> {
    parse.with_session(move || {
        let docs = collect_doc_comments(item);
        if docs.is_empty() {
            return None;
        }

        if has_explicit_natspec_tags(&docs) {
            render_natspec_docs(&docs)
        } else {
            render_plain_docs(&docs)
        }
    })
}

pub fn docs_for_item_with_inheritdoc(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    parse: &Parse,
    item: &Item<'static>,
    container: Option<&str>,
) -> Option<String> {
    parse.with_session(move || {
        let docs = collect_doc_comments(item);
        if docs.is_empty() {
            return None;
        }

        let text = db.file_input(file_id).text(db);
        let contract_name = match &item.kind {
            ItemKind::Contract(contract) => Some(contract.name.to_string()),
            _ => container.map(|name| name.to_string()),
        };
        let link_ctx = DocLinkContext {
            db,
            project_id,
            file_id,
            contract_name: contract_name.as_deref(),
        };

        if !has_explicit_natspec_tags(&docs) {
            return render_plain_docs(&docs).map(|doc| linkify_doc_text(&link_ctx, &doc));
        }

        let mut sections = if let Some(contract_name) = contract_name.as_deref() {
            let ctx = InheritdocContext {
                db,
                project_id,
                file_id,
                contract_name,
                parse,
                text: text.as_ref(),
            };
            let mut visited = HashSet::new();
            resolve_natspec_for_item_inner(&ctx, item, &mut visited).sections
        } else {
            collect_natspec_sections(&docs)
        };

        linkify_natspec_sections(&link_ctx, &mut sections);
        let rendered = render_natspec_sections(&sections);
        if rendered.is_empty() {
            None
        } else {
            Some(rendered)
        }
    })
}

fn collect_doc_comments<'a>(item: &'a Item<'static>) -> Vec<&'a DocComment<'static>> {
    item.docs.iter().collect()
}

fn render_plain_docs(docs: &[&DocComment<'_>]) -> Option<String> {
    let combined = docs
        .iter()
        .map(|doc| normalized_doc_text(doc))
        .filter(|doc| !doc.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn render_natspec_docs(docs: &[&DocComment<'_>]) -> Option<String> {
    let rendered = render_natspec_markdown(docs);
    if rendered.is_empty() {
        None
    } else {
        Some(rendered)
    }
}

fn has_explicit_natspec_tags(docs: &[&DocComment<'_>]) -> bool {
    docs.iter().any(|doc| {
        doc.natspec
            .iter()
            .any(|item| is_explicit_natspec_item(doc, item))
    })
}

fn render_natspec_markdown(docs: &[&DocComment<'_>]) -> String {
    let sections = collect_natspec_sections(docs);
    render_natspec_sections(&sections)
}

#[derive(Clone, Default)]
struct NatSpecSections {
    titles: Vec<String>,
    authors: Vec<String>,
    notices: Vec<String>,
    devs: Vec<String>,
    params: Vec<(String, String)>,
    returns: Vec<(String, String)>,
    customs: Vec<(String, String)>,
    inheritdocs: Vec<(String, String)>,
}

impl NatSpecSections {
    fn merge_missing_from(&mut self, base: &NatSpecSections) {
        if self.titles.is_empty() {
            self.titles.extend(base.titles.iter().cloned());
        }
        if self.authors.is_empty() {
            self.authors.extend(base.authors.iter().cloned());
        }
        if self.notices.is_empty() {
            self.notices.extend(base.notices.iter().cloned());
        }
        if self.devs.is_empty() {
            self.devs.extend(base.devs.iter().cloned());
        }
        for (name, content) in &base.params {
            if !self.params.iter().any(|(existing, _)| existing == name) {
                self.params.push((name.clone(), content.clone()));
            }
        }
        for (name, content) in &base.returns {
            if !self.returns.iter().any(|(existing, _)| existing == name) {
                self.returns.push((name.clone(), content.clone()));
            }
        }
        for (name, content) in &base.customs {
            if !self.customs.iter().any(|(existing, _)| existing == name) {
                self.customs.push((name.clone(), content.clone()));
            }
        }
        for (name, content) in &base.inheritdocs {
            if !self
                .inheritdocs
                .iter()
                .any(|(existing_name, existing_content)| {
                    existing_name == name && existing_content == content
                })
            {
                self.inheritdocs.push((name.clone(), content.clone()));
            }
        }
    }

    fn remove_inheritdoc(&mut self, tag: &(String, String)) {
        if let Some(index) = self
            .inheritdocs
            .iter()
            .position(|(name, content)| name == &tag.0 && content == &tag.1)
        {
            self.inheritdocs.remove(index);
        }
    }
}

fn collect_natspec_sections(docs: &[&DocComment<'_>]) -> NatSpecSections {
    let mut sections = NatSpecSections::default();
    let mut last_section: Option<LastSection> = None;

    for doc in docs {
        let explicit_items = doc
            .natspec
            .iter()
            .filter(|item| is_explicit_natspec_item(doc, item))
            .collect::<Vec<_>>();
        if explicit_items.is_empty() {
            let content = normalized_doc_text(doc);
            match last_section {
                Some(LastSection::Title(idx)) => {
                    append_paragraph_entry(&mut sections.titles, idx, content.as_str());
                }
                Some(LastSection::Author(idx)) => {
                    append_paragraph_entry(&mut sections.authors, idx, content.as_str());
                }
                Some(LastSection::Notice(idx)) => {
                    append_paragraph_entry(&mut sections.notices, idx, content.as_str());
                }
                Some(LastSection::Dev(idx)) => {
                    append_paragraph_entry(&mut sections.devs, idx, content.as_str());
                }
                Some(LastSection::Param(idx)) => {
                    append_list_entry(&mut sections.params, idx, content.as_str());
                }
                Some(LastSection::Return(idx)) => {
                    append_list_entry(&mut sections.returns, idx, content.as_str());
                }
                Some(LastSection::Custom(idx)) => {
                    append_list_entry(&mut sections.customs, idx, content.as_str());
                }
                Some(LastSection::Inheritdoc(idx)) => {
                    append_list_entry(&mut sections.inheritdocs, idx, content.as_str());
                }
                None => {
                    if !content.is_empty() {
                        let idx = push_paragraph_entry(&mut sections.notices, content.as_str());
                        last_section = Some(LastSection::Notice(idx));
                    }
                }
            }
            continue;
        }

        for item in explicit_items {
            let content = normalized_natspec_content(doc, item);
            last_section = match item.kind {
                NatSpecKind::Title => Some(LastSection::Title(push_paragraph_entry(
                    &mut sections.titles,
                    content.as_str(),
                ))),
                NatSpecKind::Author => Some(LastSection::Author(push_paragraph_entry(
                    &mut sections.authors,
                    content.as_str(),
                ))),
                NatSpecKind::Notice => Some(LastSection::Notice(push_paragraph_entry(
                    &mut sections.notices,
                    content.as_str(),
                ))),
                NatSpecKind::Dev => Some(LastSection::Dev(push_paragraph_entry(
                    &mut sections.devs,
                    content.as_str(),
                ))),
                NatSpecKind::Param { name } => Some(LastSection::Param(push_list_entry(
                    &mut sections.params,
                    name.to_string(),
                    content.as_str(),
                ))),
                NatSpecKind::Return { name } => Some(LastSection::Return(push_list_entry(
                    &mut sections.returns,
                    name.to_string(),
                    content.as_str(),
                ))),
                NatSpecKind::Custom { name } => Some(LastSection::Custom(push_list_entry(
                    &mut sections.customs,
                    name.to_string(),
                    content.as_str(),
                ))),
                NatSpecKind::Inheritdoc { contract } => {
                    Some(LastSection::Inheritdoc(push_list_entry(
                        &mut sections.inheritdocs,
                        contract.to_string(),
                        content.as_str(),
                    )))
                }
                NatSpecKind::Internal { .. } => last_section,
            };
        }
    }

    sections
}

fn render_natspec_sections(sections: &NatSpecSections) -> String {
    let mut rendered = Vec::new();
    if let Some(section) = render_paragraph_section("Title", &sections.titles) {
        rendered.push(section);
    }
    if let Some(section) = render_paragraph_section("Author", &sections.authors) {
        rendered.push(section);
    }
    if let Some(section) = render_paragraph_section("Notice", &sections.notices) {
        rendered.push(section);
    }
    if let Some(section) = render_paragraph_section("Dev", &sections.devs) {
        rendered.push(section);
    }
    if let Some(section) = render_list_section("Parameters", &sections.params) {
        rendered.push(section);
    }
    if let Some(section) = render_list_section("Returns", &sections.returns) {
        rendered.push(section);
    }
    if let Some(section) = render_list_section("Custom", &sections.customs) {
        rendered.push(section);
    }
    if let Some(section) = render_list_section("Inheritdoc", &sections.inheritdocs) {
        rendered.push(section);
    }

    rendered.join("\n\n")
}

#[derive(Clone, Copy)]
enum LastSection {
    Title(usize),
    Author(usize),
    Notice(usize),
    Dev(usize),
    Param(usize),
    Return(usize),
    Custom(usize),
    Inheritdoc(usize),
}

#[derive(Clone)]
struct ResolvedNatSpec {
    sections: NatSpecSections,
    has_explicit_tags: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InheritdocKey {
    file_id: FileId,
    contract: String,
    signature: String,
}

struct BaseContract {
    file_id: FileId,
    name: String,
}

struct InheritdocContext<'a> {
    db: &'a dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    contract_name: &'a str,
    parse: &'a Parse,
    text: &'a str,
}

fn log_inheritdoc_failure(
    ctx: &InheritdocContext<'_>,
    signature: Option<&str>,
    inheritdoc_contract: &str,
    reason: &'static str,
) {
    debug!(
        file_id = ?ctx.file_id,
        contract = %ctx.contract_name,
        signature = %signature.unwrap_or("<unknown>"),
        inheritdoc = %inheritdoc_contract,
        reason,
        "inheritdoc: resolution failed"
    );
}

fn push_paragraph_entry(section: &mut Vec<String>, content: &str) -> usize {
    section.push(normalize_natspec_text(content));
    section.len() - 1
}

fn append_paragraph_entry(section: &mut [String], idx: usize, content: &str) {
    if let Some(entry) = section.get_mut(idx) {
        append_markdown(entry, content);
    }
}

fn push_list_entry(section: &mut Vec<(String, String)>, name: String, content: &str) -> usize {
    let name = escape_inline_code(name.trim());
    section.push((name, normalize_natspec_text(content)));
    section.len() - 1
}

fn append_list_entry(section: &mut [(String, String)], idx: usize, content: &str) {
    if let Some((_, entry)) = section.get_mut(idx) {
        append_markdown(entry, content);
    }
}

fn render_paragraph_section(label: &str, entries: &[String]) -> Option<String> {
    let paragraphs = entries
        .iter()
        .map(|entry| entry.as_str())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        None
    } else {
        Some(format!("**{label}**\n{}", paragraphs.join("\n\n")))
    }
}

fn render_list_section(label: &str, entries: &[(String, String)]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let lines = entries
        .iter()
        .map(|(name, content)| format_list_item(name, content))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("**{label}**\n{lines}"))
}

fn resolve_natspec_for_item(
    ctx: &InheritdocContext<'_>,
    item: &Item<'static>,
    visited: &mut HashSet<InheritdocKey>,
) -> ResolvedNatSpec {
    ctx.parse
        .with_session(|| resolve_natspec_for_item_inner(ctx, item, visited))
}

fn resolve_natspec_for_item_inner(
    ctx: &InheritdocContext<'_>,
    item: &Item<'static>,
    visited: &mut HashSet<InheritdocKey>,
) -> ResolvedNatSpec {
    let docs = collect_doc_comments(item);
    if docs.is_empty() {
        return ResolvedNatSpec {
            sections: NatSpecSections::default(),
            has_explicit_tags: false,
        };
    }

    let has_explicit_tags = has_explicit_natspec_tags(&docs);
    if !has_explicit_tags {
        return ResolvedNatSpec {
            sections: NatSpecSections::default(),
            has_explicit_tags: false,
        };
    }

    let mut sections = collect_natspec_sections(&docs);
    let resolved_tag = sections.inheritdocs.first().cloned();
    let Some((inheritdoc_contract, _)) = resolved_tag.clone() else {
        return ResolvedNatSpec {
            sections,
            has_explicit_tags: true,
        };
    };

    let Some(signature) = item_inheritdoc_signature(ctx.parse, ctx.text, item) else {
        log_inheritdoc_failure(ctx, None, &inheritdoc_contract, "signature_unavailable");
        return ResolvedNatSpec {
            sections,
            has_explicit_tags: true,
        };
    };

    let key = InheritdocKey {
        file_id: ctx.file_id,
        contract: ctx.contract_name.to_string(),
        signature: signature.clone(),
    };
    if !visited.insert(key.clone()) {
        log_inheritdoc_failure(
            ctx,
            Some(&signature),
            &inheritdoc_contract,
            "cycle_detected",
        );
        return ResolvedNatSpec {
            sections,
            has_explicit_tags: true,
        };
    }

    if let Some(base_docs) = resolve_inheritdoc_base(
        ctx,
        &signature,
        matches!(item.kind, ItemKind::Contract(_)),
        &inheritdoc_contract,
        visited,
    ) {
        if base_docs.has_explicit_tags {
            sections.merge_missing_from(&base_docs.sections);
        }
        if let Some(tag) = resolved_tag {
            sections.remove_inheritdoc(&tag);
        }
    }

    visited.remove(&key);

    ResolvedNatSpec {
        sections,
        has_explicit_tags: true,
    }
}

fn resolve_inheritdoc_base(
    ctx: &InheritdocContext<'_>,
    signature: &str,
    is_contract: bool,
    inheritdoc_contract: &str,
    visited: &mut HashSet<InheritdocKey>,
) -> Option<ResolvedNatSpec> {
    let base = resolve_base_contract(
        ctx.db,
        ctx.project_id,
        ctx.file_id,
        ctx.contract_name,
        inheritdoc_contract,
    )?;
    let base_text = ctx.db.file_input(base.file_id).text(ctx.db);
    let base_parse = sa_syntax::parse_file(base_text.as_ref());
    let base_contract = match find_contract_in_parse(&base_parse, &base.name) {
        Some(contract) => contract,
        None => {
            debug!(
                file_id = ?ctx.file_id,
                contract = %ctx.contract_name,
                signature = %signature,
                inheritdoc = %inheritdoc_contract,
                base_contract = %base.name,
                "inheritdoc: base contract not found in parsed file"
            );
            return None;
        }
    };
    let base_item = if is_contract {
        base_contract
    } else {
        match find_contract_member_by_signature(
            &base_parse,
            base_text.as_ref(),
            &base.name,
            signature,
        ) {
            Some(item) => item,
            None => {
                debug!(
                    file_id = ?ctx.file_id,
                    contract = %ctx.contract_name,
                    signature = %signature,
                    inheritdoc = %inheritdoc_contract,
                    base_contract = %base.name,
                    "inheritdoc: base member with signature not found"
                );
                return None;
            }
        }
    };
    let base_ctx = InheritdocContext {
        db: ctx.db,
        project_id: ctx.project_id,
        file_id: base.file_id,
        contract_name: &base.name,
        parse: &base_parse,
        text: base_text.as_ref(),
    };

    Some(resolve_natspec_for_item(&base_ctx, base_item, visited))
}

fn resolve_base_contract(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    contract_name: &str,
    inheritdoc_contract: &str,
) -> Option<BaseContract> {
    let project = db.project_input(project_id);
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = match snapshot.for_file(file_id) {
        Some(snapshot) => snapshot,
        None => {
            debug!(
                file_id = ?file_id,
                contract = %contract_name,
                inheritdoc = %inheritdoc_contract,
                "inheritdoc: no sema snapshot for file"
            );
            return None;
        }
    };
    snapshot.with_gcx(|gcx| {
        let source_id = match snapshot.source_id_for_file(file_id) {
            Some(source_id) => source_id,
            None => {
                debug!(
                    file_id = ?file_id,
                    contract = %contract_name,
                    inheritdoc = %inheritdoc_contract,
                    "inheritdoc: no source id for file"
                );
                return None;
            }
        };
        let source = gcx.hir.source(source_id);
        let current_contract_id = match source.items.iter().find_map(|item_id| {
            let contract_id = item_id.as_contract()?;
            let contract = gcx.hir.contract(contract_id);
            (contract.name.as_str() == contract_name).then_some(contract_id)
        }) {
            Some(contract_id) => contract_id,
            None => {
                debug!(
                    file_id = ?file_id,
                    contract = %contract_name,
                    inheritdoc = %inheritdoc_contract,
                    "inheritdoc: contract not found in source"
                );
                return None;
            }
        };
        let contract = gcx.hir.contract(current_contract_id);
        if contract.linearized_bases.is_empty() {
            debug!(
                file_id = ?file_id,
                contract = %contract_name,
                inheritdoc = %inheritdoc_contract,
                "inheritdoc: linearized bases empty"
            );
            return None;
        }
        if !contract.bases.is_empty() && contract.linearized_bases.len() == 1 {
            debug!(
                file_id = ?file_id,
                contract = %contract_name,
                inheritdoc = %inheritdoc_contract,
                "inheritdoc: linearization failed or incomplete"
            );
        }

        for base_id in contract.linearized_bases.iter().skip(1) {
            let base = gcx.hir.contract(*base_id);
            if base.name.as_str() == inheritdoc_contract {
                let base_file_id = match snapshot.file_id_for_source(base.source) {
                    Some(base_file_id) => base_file_id,
                    None => {
                        debug!(
                            file_id = ?file_id,
                            contract = %contract_name,
                            inheritdoc = %inheritdoc_contract,
                            base_contract = %base.name.as_str(),
                            "inheritdoc: base contract file id missing"
                        );
                        return None;
                    }
                };
                return Some(BaseContract {
                    file_id: base_file_id,
                    name: base.name.as_str().to_string(),
                });
            }
        }
        debug!(
            file_id = ?file_id,
            contract = %contract_name,
            inheritdoc = %inheritdoc_contract,
            "inheritdoc: base contract not found in linearized bases"
        );
        None
    })
}

fn find_contract_in_parse<'a>(parse: &'a Parse, contract_name: &str) -> Option<&'a Item<'static>> {
    parse.with_session(|| {
        parse.tree().items.iter().find(|item| {
            if let ItemKind::Contract(contract) = &item.kind {
                contract.name.to_string() == contract_name
            } else {
                false
            }
        })
    })
}

fn find_contract_member_by_signature<'a>(
    parse: &'a Parse,
    text: &str,
    contract_name: &str,
    signature: &str,
) -> Option<&'a Item<'static>> {
    let contract = find_contract_in_parse(parse, contract_name)?;
    let ItemKind::Contract(contract) = &contract.kind else {
        return None;
    };
    contract.body.iter().find(|item| {
        item_inheritdoc_signature(parse, text, item).is_some_and(|candidate| candidate == signature)
    })
}

fn item_inheritdoc_signature(parse: &Parse, text: &str, item: &Item<'static>) -> Option<String> {
    match &item.kind {
        ItemKind::Function(function) => {
            Some(function_signature_for_inheritdoc(parse, text, function))
        }
        ItemKind::Contract(contract) => parse.with_session(|| Some(contract.name.to_string())),
        _ => parse.with_session(|| item.name().map(|ident| ident.to_string())),
    }
}

fn function_signature_for_inheritdoc(
    parse: &Parse,
    text: &str,
    function: &ItemFunction<'_>,
) -> String {
    let name = parse.with_session(|| function.header.name.map(|ident| ident.to_string()));
    let name = name.unwrap_or_else(|| function.kind.to_str().to_string());
    let params = function
        .header
        .parameters
        .vars
        .iter()
        .map(|param| {
            let ty = type_text(parse, text, &param.ty).unwrap_or_default();
            normalize_signature_type(&ty)
        })
        .collect::<Vec<_>>();
    if params.is_empty() {
        name
    } else {
        format!("{}({})", name, params.join(","))
    }
}

fn normalize_signature_type(ty: &str) -> String {
    ty.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_explicit_natspec_item(doc: &DocComment<'_>, item: &NatSpecItem) -> bool {
    if !doc_has_explicit_tag(doc) {
        return false;
    }

    match item.kind {
        NatSpecKind::Internal { .. } => false,
        NatSpecKind::Param { name }
        | NatSpecKind::Return { name }
        | NatSpecKind::Custom { name } => !name.as_str().is_empty(),
        NatSpecKind::Inheritdoc { contract } => !contract.as_str().is_empty(),
        _ => true,
    }
}

fn doc_has_explicit_tag(doc: &DocComment<'_>) -> bool {
    let text = doc.symbol.as_str();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let trimmed = match doc.kind {
            CommentKind::Line => trimmed,
            CommentKind::Block => trimmed
                .strip_prefix('*')
                .map(|rest| rest.trim_start())
                .unwrap_or(trimmed),
        };
        if trimmed.starts_with('@') {
            return true;
        }
    }
    false
}

fn normalized_doc_text(doc: &DocComment<'_>) -> String {
    match doc.kind {
        CommentKind::Line => doc.symbol.as_str().trim().to_string(),
        CommentKind::Block => normalize_block_comment_text(doc.symbol.as_str()),
    }
}

fn normalized_natspec_content(doc: &DocComment<'_>, item: &NatSpecItem) -> String {
    let content = doc.natspec_content(item);
    match doc.kind {
        CommentKind::Line => content.trim().to_string(),
        CommentKind::Block => normalize_block_comment_text(content),
    }
}

fn normalize_block_comment_text(text: &str) -> String {
    let mut lines = text
        .lines()
        .map(|line| {
            let mut trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix('*') {
                trimmed = rest;
                if trimmed.starts_with(' ') {
                    trimmed = &trimmed[1..];
                }
            }
            trimmed.trim_end().to_string()
        })
        .collect::<Vec<_>>();

    while matches!(lines.first(), Some(line) if line.is_empty()) {
        lines.remove(0);
    }
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

fn normalize_natspec_text(text: &str) -> String {
    let mut lines = text.lines().map(|line| line.trim_end()).collect::<Vec<_>>();
    while matches!(lines.first(), Some(line) if line.trim().is_empty()) {
        lines.remove(0);
    }
    while matches!(lines.last(), Some(line) if line.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn append_markdown(target: &mut String, content: &str) {
    let content = normalize_natspec_text(content);
    if content.is_empty() {
        append_blank_line(target);
        return;
    }
    if target.is_empty() {
        target.push_str(&content);
        return;
    }
    if target.ends_with("\n\n") {
        target.push_str(&content);
        return;
    }
    if should_join_with_newline(target, &content) {
        if !target.ends_with('\n') {
            target.push('\n');
        }
        target.push_str(&content);
    } else {
        target.push(' ');
        target.push_str(&content);
    }
}

fn append_blank_line(target: &mut String) {
    if target.is_empty() {
        return;
    }
    if target.ends_with("\n\n") {
        return;
    }
    if target.ends_with('\n') {
        target.push('\n');
    } else {
        target.push_str("\n\n");
    }
}

fn should_join_with_newline(existing: &str, addition: &str) -> bool {
    in_fenced_code_block(existing) || addition.contains('\n') || is_markdown_block_start(addition)
}

fn in_fenced_code_block(text: &str) -> bool {
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_block = !in_block;
        }
    }
    in_block
}

fn is_markdown_block_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with('>')
        || trimmed.starts_with('#')
    {
        return true;
    }
    starts_with_list_marker(trimmed)
}

fn starts_with_list_marker(text: &str) -> bool {
    if text.starts_with("- ") || text.starts_with("* ") || text.starts_with("+ ") {
        return true;
    }
    let bytes = text.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 {
        return false;
    }
    matches!(bytes.get(idx), Some(b'.')) && matches!(bytes.get(idx + 1), Some(b' '))
}

fn format_list_item(name: &str, content: &str) -> String {
    if content.is_empty() {
        return format!("- `{name}`");
    }
    let mut lines = content.lines();
    let first = lines.next().unwrap_or("");
    if first.is_empty() {
        let mut item = format!("- `{name}`");
        for line in lines {
            item.push('\n');
            item.push_str("  ");
            item.push_str(line);
        }
        return item;
    }
    let mut item = format!("- `{name}`: {first}");
    for line in lines {
        item.push('\n');
        item.push_str("  ");
        item.push_str(line);
    }
    item
}

struct DocLinkContext<'a> {
    db: &'a dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    contract_name: Option<&'a str>,
}

fn linkify_doc_text(ctx: &DocLinkContext<'_>, text: &str) -> String {
    let mut resolver = DocLinkResolver::new(ctx);
    resolver.linkify_text(text)
}

fn linkify_natspec_sections(ctx: &DocLinkContext<'_>, sections: &mut NatSpecSections) {
    let mut resolver = DocLinkResolver::new(ctx);
    for entry in &mut sections.titles {
        *entry = resolver.linkify_text(entry);
    }
    for entry in &mut sections.authors {
        *entry = resolver.linkify_text(entry);
    }
    for entry in &mut sections.notices {
        *entry = resolver.linkify_text(entry);
    }
    for entry in &mut sections.devs {
        *entry = resolver.linkify_text(entry);
    }
    for (_, content) in &mut sections.params {
        *content = resolver.linkify_text(content);
    }
    for (_, content) in &mut sections.returns {
        *content = resolver.linkify_text(content);
    }
    for (_, content) in &mut sections.customs {
        *content = resolver.linkify_text(content);
    }
    for (_, content) in &mut sections.inheritdocs {
        *content = resolver.linkify_text(content);
    }
}

enum DocReference {
    Unqualified(String),
    Qualified { contract: String, member: String },
}

struct DocLinkResolver<'a> {
    db: &'a dyn HirDatabase,
    file_id: FileId,
    program: sa_hir::HirProgram,
    contract_name: Option<&'a str>,
    base_contracts: Vec<BaseContract>,
    file_texts: HashMap<FileId, std::sync::Arc<str>>,
}

impl<'a> DocLinkResolver<'a> {
    fn new(ctx: &DocLinkContext<'a>) -> Self {
        let program = lowered_program(ctx.db, ctx.project_id);
        let base_contracts = ctx
            .contract_name
            .and_then(|contract| {
                linearized_base_contracts(ctx.db, ctx.project_id, ctx.file_id, contract)
            })
            .unwrap_or_default();
        Self {
            db: ctx.db,
            file_id: ctx.file_id,
            program,
            contract_name: ctx.contract_name,
            base_contracts,
            file_texts: HashMap::new(),
        }
    }

    fn linkify_text(&mut self, text: &str) -> String {
        let mut rendered = String::new();
        let mut in_fence = false;
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            if line_starts_fence(line) {
                in_fence = !in_fence;
                rendered.push_str(line);
            } else if in_fence {
                rendered.push_str(line);
            } else {
                rendered.push_str(&self.linkify_inline_code(line));
            }
            if lines.peek().is_some() || text.ends_with('\n') {
                rendered.push('\n');
            }
        }
        rendered
    }

    fn linkify_inline_code(&mut self, line: &str) -> String {
        let mut rendered = String::new();
        for (segment, is_code) in split_inline_code_segments(line) {
            if is_code {
                rendered.push_str(&segment);
            } else {
                rendered.push_str(&self.linkify_brace_refs(&segment));
            }
        }
        rendered
    }

    fn linkify_brace_refs(&mut self, text: &str) -> String {
        let mut rendered = String::new();
        let mut rest = text;
        loop {
            let Some(start) = rest.find('{') else {
                rendered.push_str(rest);
                break;
            };
            rendered.push_str(&rest[..start]);
            let remainder = &rest[start + 1..];
            let Some(end) = remainder.find('}') else {
                rendered.push_str(&rest[start..]);
                break;
            };
            let raw = &remainder[..end];
            let trimmed = raw.trim();
            let label = format!("`{{{trimmed}}}`");
            if let Some(target) = self.resolve_reference(trimmed) {
                rendered.push_str(&format!("[{label}]({target})"));
            } else {
                rendered.push_str(&format!("{{{raw}}}"));
            }
            rest = &remainder[end + 1..];
        }
        rendered
    }

    fn resolve_reference(&mut self, raw: &str) -> Option<String> {
        let reference = parse_doc_reference(raw)?;
        let entry = match reference {
            DocReference::Qualified { contract, member } => {
                let contract_entry = self.resolve_contract_entry(&contract)?;
                self.resolve_member_in_contract(
                    &contract,
                    &member,
                    Some(contract_entry.location().file_id()),
                )
            }
            DocReference::Unqualified(name) => self.resolve_unqualified_reference(&name),
        }?;
        self.entry_link_target(&entry)
    }

    fn resolve_unqualified_reference(&mut self, name: &str) -> Option<DefEntry> {
        if let Some(contract_name) = self.contract_name {
            if let Some(entry) =
                self.resolve_member_in_contract(contract_name, name, Some(self.file_id))
            {
                return Some(entry);
            }
            for base in &self.base_contracts {
                if let Some(entry) =
                    self.resolve_member_in_contract(&base.name, name, Some(base.file_id))
                {
                    return Some(entry);
                }
            }
        }

        if let Some(contract_entry) = self.resolve_contract_entry(name) {
            return Some(contract_entry);
        }

        self.resolve_top_level_entry(name)
    }

    fn resolve_contract_entry(&self, name: &str) -> Option<DefEntry> {
        let mut candidates = self
            .program
            .def_map()
            .entries()
            .iter()
            .filter(|entry| entry.kind() == DefKind::Contract && entry.location().name() == name)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }

        if let Some(base) = self.base_contracts.iter().find(|base| base.name == name)
            && let Some(entry) = candidates
                .iter()
                .find(|entry| entry.location().file_id() == base.file_id)
        {
            return Some((*entry).clone());
        }

        if let Some(entry) = candidates
            .iter()
            .find(|entry| entry.location().file_id() == self.file_id)
        {
            return Some((*entry).clone());
        }

        if candidates.len() == 1 {
            return Some((*candidates.remove(0)).clone());
        }

        None
    }

    fn resolve_member_in_contract(
        &self,
        contract: &str,
        member: &str,
        preferred_file_id: Option<FileId>,
    ) -> Option<DefEntry> {
        let mut candidates = self
            .program
            .def_map()
            .entries()
            .iter()
            .filter(|entry| {
                entry.container() == Some(contract) && entry.location().name() == member
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        if let Some(preferred_file_id) = preferred_file_id
            && let Some(entry) = candidates
                .iter()
                .find(|entry| entry.location().file_id() == preferred_file_id)
        {
            return Some((*entry).clone());
        }
        if candidates.len() == 1 {
            return Some((*candidates.remove(0)).clone());
        }
        None
    }

    fn resolve_top_level_entry(&self, name: &str) -> Option<DefEntry> {
        let mut candidates = self
            .program
            .def_map()
            .entries()
            .iter()
            .filter(|entry| entry.container().is_none() && entry.location().name() == name)
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            return Some((*candidates.remove(0)).clone());
        }
        None
    }

    fn entry_link_target(&mut self, entry: &DefEntry) -> Option<String> {
        let path = self.db.file_path(entry.location().file_id());
        let uri = Url::from_file_path(PathBuf::from(path.as_str())).ok()?;
        let text = self.file_text(entry.location().file_id());
        let range = entry.location().range();
        let lsp_range = sa_span::lsp::to_lsp_range(range, text.as_ref());
        let line = lsp_range.start.line + 1;
        Some(format!("{uri}#L{line}"))
    }

    fn file_text(&mut self, file_id: FileId) -> std::sync::Arc<str> {
        if let Some(text) = self.file_texts.get(&file_id) {
            return text.clone();
        }
        let text = self.db.file_input(file_id).text(self.db).clone();
        self.file_texts.insert(file_id, text.clone());
        text
    }
}

fn line_starts_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn split_inline_code_segments(line: &str) -> Vec<(String, bool)> {
    let mut segments = Vec::new();
    let mut idx = 0;
    let bytes = line.as_bytes();
    let mut code_delimiter: Option<usize> = None;
    let mut segment_start = 0;

    while idx < bytes.len() {
        if bytes[idx] == b'`' {
            let mut run_len = 1;
            while idx + run_len < bytes.len() && bytes[idx + run_len] == b'`' {
                run_len += 1;
            }
            if code_delimiter.is_none() {
                if segment_start < idx {
                    segments.push((line[segment_start..idx].to_string(), false));
                }
                code_delimiter = Some(run_len);
                segment_start = idx;
            } else if code_delimiter == Some(run_len) {
                let end = idx + run_len;
                segments.push((line[segment_start..end].to_string(), true));
                code_delimiter = None;
                segment_start = end;
            }
            idx += run_len;
            continue;
        }
        idx += 1;
    }

    if segment_start < line.len() {
        segments.push((line[segment_start..].to_string(), code_delimiter.is_some()));
    }

    segments
}

fn parse_doc_reference(raw: &str) -> Option<DocReference> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(|ch| ch.is_whitespace()) {
        return None;
    }

    if let Some((contract, member)) = split_reference(trimmed) {
        if is_identifier_like(contract) && is_identifier_like(member) {
            return Some(DocReference::Qualified {
                contract: contract.to_string(),
                member: member.to_string(),
            });
        }
        return None;
    }

    if is_identifier_like(trimmed) {
        return Some(DocReference::Unqualified(trimmed.to_string()));
    }
    None
}

fn split_reference(value: &str) -> Option<(&str, &str)> {
    let separators = ["::", ".", "-"];
    for separator in separators {
        if let Some(index) = value.find(separator) {
            let left = &value[..index];
            let right = &value[index + separator.len()..];
            if !left.is_empty() && !right.is_empty() {
                return Some((left, right));
            }
        }
    }
    None
}

fn is_identifier_like(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn linearized_base_contracts(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    contract_name: &str,
) -> Option<Vec<BaseContract>> {
    let project = db.project_input(project_id);
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(file_id)?;
    snapshot.with_gcx(|gcx| {
        let source_id = snapshot.source_id_for_file(file_id)?;
        let source = gcx.hir.source(source_id);
        let current_contract_id = source.items.iter().find_map(|item_id| {
            let contract_id = item_id.as_contract()?;
            let contract = gcx.hir.contract(contract_id);
            (contract.name.as_str() == contract_name).then_some(contract_id)
        })?;
        let contract = gcx.hir.contract(current_contract_id);
        if contract.linearized_bases.is_empty() {
            return Some(Vec::new());
        }
        let mut bases = Vec::new();
        for base_id in contract.linearized_bases.iter().skip(1) {
            let base = gcx.hir.contract(*base_id);
            let base_file_id = snapshot.file_id_for_source(base.source)?;
            bases.push(BaseContract {
                file_id: base_file_id,
                name: base.name.as_str().to_string(),
            });
        }
        Some(bases)
    })
}

fn escape_inline_code(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '`' => {
                escaped.push('\\');
                escaped.push('`');
            }
            '\\' => {
                escaped.push('\\');
                escaped.push('\\');
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_item<'a>(parse: &'a Parse, name: &str) -> &'a Item<'static> {
        parse.with_session(|| {
            parse
                .tree()
                .items
                .iter()
                .find(|item| match &item.kind {
                    ItemKind::Contract(contract) => contract.name.to_string() == name,
                    _ => false,
                })
                .expect("contract item")
        })
    }

    fn function_item<'a>(
        parse: &'a Parse,
        contract_name: &str,
        function_name: &str,
    ) -> &'a ItemFunction<'static> {
        parse.with_session(|| {
            let contract = parse
                .tree()
                .items
                .iter()
                .find_map(|item| match &item.kind {
                    ItemKind::Contract(contract) if contract.name.to_string() == contract_name => {
                        Some(contract)
                    }
                    _ => None,
                })
                .expect("contract");
            contract
                .body
                .iter()
                .find_map(|item| match &item.kind {
                    ItemKind::Function(function) => {
                        let name = function.header.name.map(|ident| ident.to_string());
                        (name.as_deref() == Some(function_name)).then_some(function)
                    }
                    _ => None,
                })
                .expect("function")
        })
    }

    #[test]
    fn find_item_by_name_range_handles_contracts_and_members() {
        let text = r#"
pragma solidity ^0.8.20;

contract Alpha {
    function foo() public {}
}

contract Beta {}
"#;
        let parse = sa_syntax::parse_file(text);
        let alpha = contract_item(&parse, "Alpha");
        let beta = contract_item(&parse, "Beta");
        let foo_item = parse.with_session(|| {
            let ItemKind::Contract(alpha_contract) = &alpha.kind else {
                unreachable!("alpha contract")
            };
            alpha_contract
                .body
                .iter()
                .find(|item| item.name().map(|ident| ident.to_string()).as_deref() == Some("foo"))
                .expect("foo item")
        });
        let alpha_range = parse
            .span_to_text_range(parse.with_session(|| alpha.name().expect("alpha name").span))
            .expect("alpha range");
        let foo_range = parse
            .span_to_text_range(parse.with_session(|| foo_item.name().expect("foo name").span))
            .expect("foo range");
        let beta_range = parse
            .span_to_text_range(parse.with_session(|| beta.name().expect("beta name").span))
            .expect("beta range");

        let alpha_item =
            find_item_by_name_range(&parse, None, alpha_range).expect("alpha item by range");
        let alpha_name = parse.with_session(|| alpha_item.name().unwrap().to_string());
        assert_eq!(alpha_name, "Alpha");

        let foo_item =
            find_item_by_name_range(&parse, Some("Alpha"), foo_range).expect("foo item by range");
        let foo_name = parse.with_session(|| foo_item.name().unwrap().to_string());
        assert_eq!(foo_name, "foo");

        assert!(find_item_by_name_range(&parse, None, foo_range).is_none());

        let beta_item =
            find_item_by_name_range(&parse, None, beta_range).expect("beta item by range");
        let beta_name = parse.with_session(|| beta_item.name().unwrap().to_string());
        assert_eq!(beta_name, "Beta");
    }

    #[test]
    fn format_function_signature_includes_params_and_returns() {
        let text = r#"
pragma solidity ^0.8.20;

contract Alpha {
    function foo(uint256 a, address b) external returns (uint256, bool ok) {}
}
"#;
        let parse = sa_syntax::parse_file(text);
        let function = function_item(&parse, "Alpha", "foo");
        let signature = format_function_signature(&parse, text, function);
        assert_eq!(
            signature,
            "function foo(uint256 a, address b) returns (uint256, bool ok)"
        );
    }

    #[test]
    fn docs_for_item_handles_plain_and_natspec() {
        let plain = r#"
/// First line
/// Second line
contract Plain {}
"#;
        let parse = sa_syntax::parse_file(plain);
        let contract = contract_item(&parse, "Plain");
        let docs = docs_for_item(&parse, contract).expect("plain docs");
        assert_eq!(docs, "First line\nSecond line");

        let tagged = r#"
/**
 * @notice Hello
 * @dev Details
 */
contract Tagged {}
"#;
        let parse = sa_syntax::parse_file(tagged);
        let contract = contract_item(&parse, "Tagged");
        let docs = docs_for_item(&parse, contract).expect("tagged docs");
        assert!(docs.contains("**Notice**"));
        assert!(docs.contains("Hello"));
        assert!(docs.contains("**Dev**"));
        assert!(docs.contains("Details"));
    }

    #[test]
    fn normalize_block_comment_text_trims_and_strips_stars() {
        let text = "\n * hello \n *\n * world \n ";
        assert_eq!(normalize_block_comment_text(text), "hello\n\nworld");
    }

    #[test]
    fn append_markdown_respects_newlines_and_blank_lines() {
        let mut target = "Hello".to_string();
        append_markdown(&mut target, "world");
        assert_eq!(target, "Hello world");

        let mut target = "Hello".to_string();
        append_markdown(&mut target, "line1\nline2");
        assert_eq!(target, "Hello\nline1\nline2");

        let mut target = "Hello".to_string();
        append_markdown(&mut target, "");
        assert_eq!(target, "Hello\n\n");
    }

    #[test]
    fn fenced_code_detection_and_inline_segments() {
        assert!(line_starts_fence("```solidity"));
        assert!(line_starts_fence("   ~~~"));
        assert!(!line_starts_fence("`inline`"));

        assert!(!in_fenced_code_block("```solidity\ncode\n```"));
        assert!(in_fenced_code_block("```solidity\ncode\n"));

        let segments = split_inline_code_segments("hello `code` world");
        assert_eq!(
            segments,
            vec![
                ("hello ".to_string(), false),
                ("`code`".to_string(), true),
                (" world".to_string(), false),
            ]
        );

        let segments = split_inline_code_segments("hello `code");
        assert_eq!(
            segments,
            vec![("hello ".to_string(), false), ("`code".to_string(), true)]
        );
    }

    #[test]
    fn doc_reference_parsing_accepts_valid_and_rejects_invalid() {
        assert!(matches!(
            parse_doc_reference("Contract::member"),
            Some(DocReference::Qualified { contract, member })
                if contract == "Contract" && member == "member"
        ));
        assert!(matches!(
            parse_doc_reference("Contract.member"),
            Some(DocReference::Qualified { contract, member })
                if contract == "Contract" && member == "member"
        ));
        assert!(matches!(
            parse_doc_reference("Contract-member"),
            Some(DocReference::Qualified { contract, member })
                if contract == "Contract" && member == "member"
        ));
        assert!(matches!(
            parse_doc_reference("Thing"),
            Some(DocReference::Unqualified(name)) if name == "Thing"
        ));
        assert!(parse_doc_reference("not valid").is_none());
        assert!(parse_doc_reference("1bad").is_none());
    }

    #[test]
    fn markdown_block_detection_and_list_items() {
        assert!(is_markdown_block_start("- item"));
        assert!(is_markdown_block_start("1. item"));
        assert!(is_markdown_block_start("> quote"));
        assert!(!is_markdown_block_start("plain text"));

        assert_eq!(format_list_item("name", ""), "- `name`");
        assert_eq!(
            format_list_item("name", "first\nsecond"),
            "- `name`: first\n  second"
        );
        assert_eq!(
            format_list_item("name", "\nindented"),
            "- `name`\n  indented"
        );
    }

    #[test]
    fn escape_inline_code_escapes_backticks_and_backslashes() {
        assert_eq!(escape_inline_code("`code`"), "\\`code\\`");
        assert_eq!(escape_inline_code("path\\file"), "path\\\\file");
    }
}
