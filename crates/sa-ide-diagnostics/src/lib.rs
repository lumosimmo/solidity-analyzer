use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use forge_lint::linter::{EarlyLintVisitor, LateLintVisitor, Lint, LintContext, LinterConfig};
use forge_lint::sol::{codesize, gas, high, info, med};
use foundry_common::comments::{
    Comments,
    inline_config::{InlineConfig, InlineConfigItem},
};
use foundry_compilers::ProjectPathsConfig;
use sa_config::{ResolvedFoundryConfig, solar_opts_from_config};
use sa_ide_assists::is_fixable_lint;
use sa_paths::NormalizedPath;
use sa_project_model::project_paths_from_config;
use sa_sema::VfsOverlayFileLoader;
use sa_span::{TextRange, TextSize};
use sa_vfs::VfsSnapshot;
use solar::ast;
use solar::ast::visit::Visit as _;
use solar::interface::diagnostics::{Diag, DiagCtxt, InMemoryEmitter, Level};
use solar::interface::source_map::{FileName, SourceFile};
use solar::interface::{Session, SourceMap};
use solar::sema::Compiler;
use solar::sema::hir::Visit as _;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub file_path: NormalizedPath,
    pub range: TextRange,
    pub severity: DiagnosticSeverity,
    pub code: Option<String>,
    pub source: DiagnosticSource,
    pub fixable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSource {
    Solc,
    Solar,
    ForgeLint,
}

impl DiagnosticSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticSource::Solc => "solc",
            DiagnosticSource::Solar => "solar",
            DiagnosticSource::ForgeLint => "forge-lint",
        }
    }
}

pub fn collect_solar_lints(
    config: &ResolvedFoundryConfig,
    files: &[PathBuf],
) -> Result<Vec<Diagnostic>> {
    collect_solar_lints_inner(config, files, None)
}

pub fn collect_solar_lints_with_overlay(
    config: &ResolvedFoundryConfig,
    files: &[PathBuf],
    snapshot: &VfsSnapshot,
) -> Result<Vec<Diagnostic>> {
    collect_solar_lints_inner(config, files, Some(snapshot))
}

fn collect_solar_lints_inner(
    config: &ResolvedFoundryConfig,
    files: &[PathBuf],
    snapshot: Option<&VfsSnapshot>,
) -> Result<Vec<Diagnostic>> {
    let (emitter, buffer) = InMemoryEmitter::new();
    let dcx = DiagCtxt::new(Box::new(emitter));
    let source_map = Arc::new(SourceMap::empty());
    if let Some(snapshot) = snapshot {
        source_map.set_file_loader(VfsOverlayFileLoader::new(snapshot.clone()));
    }
    let opts = solar_opts_from_config(config);
    let session = Session::builder()
        .dcx(dcx)
        .source_map(Arc::clone(&source_map))
        .opts(opts)
        .build();
    let mut compiler = Compiler::new(session);

    let root = PathBuf::from(config.workspace().root().as_str());
    let absolute_files = files
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .collect::<Vec<_>>();

    // Intentionally ignore parse results; diagnostics are buffered and collected later, so we do
    // not fail fast based on this return value.
    let _parse_result = compiler.enter_mut(
        |compiler| -> Result<(), solar::interface::diagnostics::ErrorGuaranteed> {
            let mut parser = compiler.parse();
            parser.load_files(absolute_files.iter())?;
            parser.parse();
            let _ = compiler.lower_asts()?;
            Ok(())
        },
    );

    let path_config =
        project_paths_from_config(config.workspace(), config.active_profile().remappings())?;
    let lint_ids = collect_lint_ids();
    let lint_id_set = lint_ids
        .iter()
        .map(|id| id.to_lowercase())
        .collect::<HashSet<_>>();
    // Intentionally ignore lint results; diagnostics are buffered and collected later, so we do
    // not fail fast based on this return value.
    let _lint_result = compiler.enter(
        |compiler| -> Result<(), solar::interface::diagnostics::ErrorGuaranteed> {
            let gcx = compiler.gcx();
            for path in &absolute_files {
                let Some((_, ast_source)) = gcx.get_ast_source(path) else {
                    continue;
                };
                let Some(ast) = &ast_source.ast else {
                    continue;
                };
                let file = &ast_source.file;
                let inline_config = parse_inline_config(compiler.sess(), file, ast, &lint_ids);
                run_early_lints(compiler.sess(), &path_config, path, ast, &inline_config)?;

                let Some((source_id, _)) = gcx.get_hir_source(path) else {
                    continue;
                };
                run_late_lints(compiler.sess(), gcx, source_id, &inline_config)?;
            }
            Ok(())
        },
    );

    Ok(buffer
        .read()
        .iter()
        .filter_map(|diag| solar_diag_to_diagnostic(compiler.sess(), diag, &lint_id_set))
        .collect())
}

fn collect_lint_ids() -> Vec<&'static str> {
    let mut ids = Vec::new();
    ids.extend(high::REGISTERED_LINTS.iter().map(|lint| lint.id()));
    ids.extend(med::REGISTERED_LINTS.iter().map(|lint| lint.id()));
    ids.extend(info::REGISTERED_LINTS.iter().map(|lint| lint.id()));
    ids.extend(gas::REGISTERED_LINTS.iter().map(|lint| lint.id()));
    ids.extend(codesize::REGISTERED_LINTS.iter().map(|lint| lint.id()));
    ids
}

fn parse_inline_config<'ast>(
    sess: &Session,
    file: &SourceFile,
    ast: &'ast ast::SourceUnit<'ast>,
    lint_ids: &[&str],
) -> InlineConfig<Vec<String>> {
    let comments = Comments::new(file, sess.source_map(), false, false, None);
    let items = comments.iter().filter_map(|comment| {
        let mut item = comment.lines.first()?.as_str();
        if let Some(prefix) = comment.prefix() {
            item = item.strip_prefix(prefix).unwrap_or(item);
        }
        if let Some(suffix) = comment.suffix() {
            item = item.strip_suffix(suffix).unwrap_or(item);
        }
        let item = item.trim_start().strip_prefix("forge-lint:")?.trim();
        let span = comment.span;
        match InlineConfigItem::parse(item, lint_ids) {
            Ok(item) => Some((span, item)),
            Err(error) => {
                sess.dcx.warn(error.to_string()).span(span).emit();
                None
            }
        }
    });

    InlineConfig::from_ast(items, ast, sess.source_map())
}

pub fn merge_diagnostics(solc: Vec<Diagnostic>, solar: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut solc_sorted = solc;
    let mut solar_sorted = solar;
    sort_diagnostics(&mut solc_sorted);
    sort_diagnostics(&mut solar_sorted);

    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for diag in solc_sorted.into_iter().chain(solar_sorted) {
        let key = DiagnosticKey::new(&diag);
        if seen.insert(key) {
            merged.push(diag);
        }
    }
    merged
}

fn run_early_lints<'a>(
    sess: &'a Session,
    path_config: &ProjectPathsConfig,
    path: &Path,
    ast: &'a ast::SourceUnit<'a>,
    inline_config: &InlineConfig<Vec<String>>,
) -> Result<(), solar::interface::diagnostics::ErrorGuaranteed> {
    let mut passes_and_lints = Vec::new();
    passes_and_lints.extend(high::create_early_lint_passes());
    passes_and_lints.extend(med::create_early_lint_passes());
    passes_and_lints.extend(info::create_early_lint_passes());

    if !path_config.is_test_or_script(path) {
        passes_and_lints.extend(gas::create_early_lint_passes());
        passes_and_lints.extend(codesize::create_early_lint_passes());
    }

    let (mut passes, lints): (Vec<_>, Vec<_>) = passes_and_lints.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut passes, mut ids), (pass, lints)| {
            passes.push(pass);
            ids.extend(lints.iter().map(|lint| lint.id()));
            (passes, ids)
        },
    );

    let ctx = LintContext::new(
        sess,
        true,
        false,
        LinterConfig {
            inline: inline_config,
            mixed_case_exceptions: &[],
        },
        lints,
    );
    let mut visitor = EarlyLintVisitor::new(&ctx, &mut passes);
    _ = visitor.visit_source_unit(ast);
    visitor.post_source_unit(ast);
    Ok(())
}

fn run_late_lints<'gcx>(
    sess: &'gcx Session,
    gcx: solar::sema::Gcx<'gcx>,
    source_id: solar::sema::hir::SourceId,
    inline_config: &InlineConfig<Vec<String>>,
) -> Result<(), solar::interface::diagnostics::ErrorGuaranteed> {
    let mut passes_and_lints = Vec::new();
    passes_and_lints.extend(high::create_late_lint_passes());
    passes_and_lints.extend(med::create_late_lint_passes());
    passes_and_lints.extend(info::create_late_lint_passes());

    let (mut passes, lints): (Vec<_>, Vec<_>) = passes_and_lints.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut passes, mut ids), (pass, lints)| {
            passes.push(pass);
            ids.extend(lints.iter().map(|lint| lint.id()));
            (passes, ids)
        },
    );

    let ctx = LintContext::new(
        sess,
        true,
        false,
        LinterConfig {
            inline: inline_config,
            mixed_case_exceptions: &[],
        },
        lints,
    );

    let hir = &gcx.hir;
    let mut visitor = LateLintVisitor::new(&ctx, &mut passes, hir);
    let _ = visitor.visit_nested_source(source_id);
    Ok(())
}

fn solar_diag_to_diagnostic(
    session: &Session,
    diag: &Diag,
    lint_ids: &HashSet<String>,
) -> Option<Diagnostic> {
    let span = diag.span.primary_span()?;
    if span.is_dummy() {
        return None;
    }
    let range = session.source_map().span_to_range(span).ok()?;
    let start = TextSize::try_from(range.start).ok()?;
    let end = TextSize::try_from(range.end).ok()?;
    let (source_file, _) = session.source_map().span_to_location_info(span);
    let source_file = source_file?;
    let file_path = file_name_to_path(&source_file.name)?;

    let code = diag.id().map(|code| code.to_lowercase());
    let source = match code.as_deref() {
        Some(code) if lint_ids.contains(code) => DiagnosticSource::ForgeLint,
        _ => DiagnosticSource::Solar,
    };
    let fixable = code.as_deref().is_some_and(is_fixable_lint);

    Some(Diagnostic {
        file_path,
        range: TextRange::new(start, end),
        severity: level_to_severity(diag.level()),
        code,
        source,
        fixable,
        message: diag.label().to_string(),
    })
}

fn level_to_severity(level: Level) -> DiagnosticSeverity {
    match level {
        Level::Bug | Level::Fatal | Level::Error => DiagnosticSeverity::Error,
        Level::Warning => DiagnosticSeverity::Warning,
        Level::Note
        | Level::OnceNote
        | Level::Help
        | Level::OnceHelp
        | Level::FailureNote
        | Level::Allow => DiagnosticSeverity::Info,
    }
}

fn file_name_to_path(file_name: &FileName) -> Option<NormalizedPath> {
    match file_name {
        FileName::Real(path) => Some(NormalizedPath::new(path.to_string_lossy())),
        FileName::Custom(name) => Some(NormalizedPath::new(name)),
        FileName::Stdin => None,
    }
}

fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        (
            a.file_path.as_str(),
            a.range.start().raw(),
            a.range.end().raw(),
            &a.code,
            &a.message,
        )
            .cmp(&(
                b.file_path.as_str(),
                b.range.start().raw(),
                b.range.end().raw(),
                &b.code,
                &b.message,
            ))
    });
}

#[derive(Hash, PartialEq, Eq)]
struct DiagnosticKey {
    file_path: NormalizedPath,
    range: TextRange,
    code: Option<String>,
}

impl DiagnosticKey {
    fn new(diag: &Diagnostic) -> Self {
        Self {
            file_path: diag.file_path.clone(),
            range: diag.range,
            code: diag.code.clone(),
        }
    }
}
