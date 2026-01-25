use std::sync::Arc;

use sa_span::{TextRange, TextSize};
use solar_ast as ast;
use solar_interface::diagnostics::{Diag, DiagCtxt, InMemoryEmitter};
use solar_interface::source_map::FileName;
use solar_interface::{Session, Span};
use solar_parse::Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    message: String,
}

impl SyntaxError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn from_diag(diag: &Diag) -> Self {
        Self::new(diag.label().to_string())
    }
}

pub type SyntaxTree = ast::SourceUnit<'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImport {
    pub path: String,
    pub items: ParsedImportItems,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAlias {
    pub name: String,
    pub alias: Option<String>,
}

impl ImportAlias {
    pub fn local_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedImportItems {
    Plain,
    SourceAlias(String),
    Glob(String),
    Aliases(Vec<ImportAlias>),
}

pub struct Parse {
    _arena: ast::Arena,
    tree: SyntaxTree,
    errors: Vec<SyntaxError>,
    session: Session,
}

impl Parse {
    pub fn tree(&self) -> &SyntaxTree {
        &self.tree
    }

    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    pub fn with_session<R>(&self, f: impl FnOnce() -> R) -> R {
        self.session.enter_sequential(f)
    }

    pub fn span_to_text_range(&self, span: Span) -> Option<TextRange> {
        self.with_session(|| {
            let range = self.session.source_map().span_to_range(span).ok()?;
            let start = TextSize::try_from(range.start).ok()?;
            let end = TextSize::try_from(range.end).ok()?;
            Some(TextRange::new(start, end))
        })
    }
}

pub fn parse_file(text: &str) -> Parse {
    let arena = ast::Arena::new();
    let (emitter, buffer) = InMemoryEmitter::new();
    let dcx = DiagCtxt::new(Box::new(emitter));
    let session = Session::builder().dcx(dcx).build();

    let tree = session.enter_sequential(|| {
        let filename = FileName::Custom("input.sol".into());
        let parser = Parser::from_source_code(&session, &arena, filename, text.to_string());
        match parser {
            Ok(mut parser) => match parser.parse_file() {
                Ok(tree) => tree,
                Err(err) => {
                    err.emit();
                    ast::SourceUnit::new(Default::default())
                }
            },
            Err(_) => ast::SourceUnit::new(Default::default()),
        }
    });

    let errors = collect_errors(Arc::clone(&buffer));
    // SAFETY: the arena is stored in Parse, so the tree's references stay valid.
    let tree =
        unsafe { std::mem::transmute::<ast::SourceUnit<'_>, ast::SourceUnit<'static>>(tree) };

    Parse {
        _arena: arena,
        tree,
        errors,
        session,
    }
}

fn collect_errors(buffer: Arc<solar_data_structures::sync::RwLock<Vec<Diag>>>) -> Vec<SyntaxError> {
    let guard = buffer.read();
    guard
        .iter()
        .filter(|diag| diag.is_error())
        .map(SyntaxError::from_diag)
        .collect()
}

/// Parses the import paths from Solidity source text.
///
/// This is a convenience function that extracts all import directive paths
/// from the given source text. It returns an empty vector if parsing fails.
pub fn parse_imports(text: &str) -> Vec<String> {
    parse_imports_with_items(text)
        .into_iter()
        .map(|import| import.path)
        .collect()
}

/// Parses import directives from Solidity source text.
///
/// This returns each import path with the import item shape to enable alias-aware resolution.
pub fn parse_imports_with_items(text: &str) -> Vec<ParsedImport> {
    let parse = parse_file(text);
    parse.with_session(|| {
        parse
            .tree()
            .imports()
            .map(|(_, directive)| {
                let path = directive.path.value.as_str().to_string();
                let items = match &directive.items {
                    ast::ImportItems::Plain(alias) => match alias {
                        Some(alias) => ParsedImportItems::SourceAlias(alias.as_str().to_string()),
                        None => ParsedImportItems::Plain,
                    },
                    ast::ImportItems::Aliases(aliases) => {
                        let aliases = aliases
                            .iter()
                            .map(|(name, alias)| ImportAlias {
                                name: name.as_str().to_string(),
                                alias: alias.as_ref().map(|alias| alias.as_str().to_string()),
                            })
                            .collect();
                        ParsedImportItems::Aliases(aliases)
                    }
                    ast::ImportItems::Glob(alias) => {
                        ParsedImportItems::Glob(alias.as_str().to_string())
                    }
                };
                ParsedImport { path, items }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ImportAlias, ParsedImportItems, parse_file, parse_imports, parse_imports_with_items,
    };

    #[test]
    fn parses_valid_solidity() {
        let parse = parse_file("contract Foo {}");
        assert!(parse.errors().is_empty());
        assert_eq!(parse.tree().count_contracts(), 1);
    }

    #[test]
    fn parses_invalid_solidity_with_errors() {
        let parse = parse_file("contract {");
        assert!(!parse.errors().is_empty());
    }

    #[test]
    fn parses_empty_file() {
        let parse = parse_file("");
        assert!(parse.errors().is_empty());
        assert_eq!(parse.tree().count_contracts(), 0);
    }

    #[test]
    fn parse_imports_extracts_import_paths() {
        let text = r#"
import "lib/Foo.sol";
import "./Bar.sol";
import "../Baz.sol";

contract Main {}
"#;
        let imports = parse_imports(text);
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0], "lib/Foo.sol");
        assert_eq!(imports[1], "./Bar.sol");
        assert_eq!(imports[2], "../Baz.sol");
    }

    #[test]
    fn parse_imports_returns_empty_for_no_imports() {
        let text = "contract Main {}";
        let imports = parse_imports(text);
        assert!(imports.is_empty());
    }

    #[test]
    fn parse_imports_whitespace_variants() {
        // Leading/trailing whitespace and multiple spaces
        let text = r#"
    import   "lib/A.sol"  ;
		import	"lib/B.sol";
import "lib/C.sol"   ;

contract Main {}
"#;
        let imports = parse_imports(text);
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0], "lib/A.sol");
        assert_eq!(imports[1], "lib/B.sol");
        assert_eq!(imports[2], "lib/C.sol");
    }

    #[test]
    fn parse_imports_unusual_paths() {
        let text = r#"
import "./relative.sol";
import "../parent/file.sol";
import "../../grandparent.sol";
import "/absolute/path.sol";
import "https://example.com/Contract.sol";
import "ipfs://QmHash/File.sol";
import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "forge-std/Test.sol";

contract Main {}
"#;
        let imports = parse_imports(text);
        assert_eq!(imports.len(), 8);
        assert_eq!(imports[0], "./relative.sol");
        assert_eq!(imports[1], "../parent/file.sol");
        assert_eq!(imports[2], "../../grandparent.sol");
        assert_eq!(imports[3], "/absolute/path.sol");
        assert_eq!(imports[4], "https://example.com/Contract.sol");
        assert_eq!(imports[5], "ipfs://QmHash/File.sol");
        assert_eq!(imports[6], "@openzeppelin/contracts/token/ERC20/ERC20.sol");
        assert_eq!(imports[7], "forge-std/Test.sol");
    }

    #[test]
    fn parse_imports_malformed_syntax() {
        // Missing quotes - parser should recover and not extract
        let text1 = r#"
import lib/NoQuotes.sol;
contract Main {}
"#;
        let imports1 = parse_imports(text1);
        // Parser may or may not extract this depending on recovery
        // The important thing is it doesn't panic
        let _ = imports1;

        // Missing semicolon - parser should still extract the path
        let text2 = r#"
import "lib/NoSemicolon.sol"
contract Main {}
"#;
        let imports2 = parse_imports(text2);
        // Parser should recover and extract the path (or not, depending on recovery)
        // The important thing is it doesn't panic
        let _ = imports2;

        // Stray characters - should not crash
        let text3 = r#"
import "lib/Valid.sol";
import , "lib/StrayComma.sol";
contract Main {}
"#;
        let imports3 = parse_imports(text3);
        // Parser recovery behavior may vary - the important thing is no panic
        // and we get some reasonable output
        let _ = imports3;

        // Completely broken syntax - should not crash
        let text4 = "import import import";
        let imports4 = parse_imports(text4);
        let _ = imports4;
    }

    #[test]
    fn parse_imports_empty_path() {
        // Empty string path
        let text = r#"
import "";
import "lib/Valid.sol";

contract Main {}
"#;
        let imports = parse_imports(text);
        // Both should be extracted - empty path is still a valid syntax element
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0], "");
        assert_eq!(imports[1], "lib/Valid.sol");
    }

    #[test]
    fn parse_imports_with_aliases_and_symbols() {
        let text = r#"
import "lib/Full.sol";
import {Symbol} from "lib/Named.sol";
import {A, B, C} from "lib/Multiple.sol";
import * as Lib from "lib/Wildcard.sol";
import "lib/Simple.sol" as Simple;

contract Main {}
"#;
        let imports = parse_imports(text);
        assert_eq!(imports.len(), 5);
        assert_eq!(imports[0], "lib/Full.sol");
        assert_eq!(imports[1], "lib/Named.sol");
        assert_eq!(imports[2], "lib/Multiple.sol");
        assert_eq!(imports[3], "lib/Wildcard.sol");
        assert_eq!(imports[4], "lib/Simple.sol");
    }

    #[test]
    fn parse_imports_with_items_records_alias_shapes() {
        let text = r#"
import {Lib as Other, Util} from "./Lib.sol";
import * as Glob from "./Glob.sol";
import "./Plain.sol";
import "./Alias.sol" as Alias;
"#;
        let imports = parse_imports_with_items(text);
        assert_eq!(imports.len(), 4);

        if let ParsedImportItems::Aliases(aliases) = &imports[0].items {
            assert_eq!(
                aliases,
                &vec![
                    ImportAlias {
                        name: "Lib".to_string(),
                        alias: Some("Other".to_string()),
                    },
                    ImportAlias {
                        name: "Util".to_string(),
                        alias: None,
                    },
                ]
            );
        } else {
            panic!("expected aliases import");
        }

        assert!(matches!(
            imports[1].items,
            ParsedImportItems::Glob(ref name) if name == "Glob"
        ));
        assert!(matches!(imports[2].items, ParsedImportItems::Plain));
        assert!(matches!(
            imports[3].items,
            ParsedImportItems::SourceAlias(ref name) if name == "Alias"
        ));
    }
}
