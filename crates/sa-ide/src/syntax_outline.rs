use sa_span::{TextRange, TextSize};
use sa_syntax::{Parse, SyntaxTree, ast};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Contract,
    Function,
    Struct,
    Enum,
    Event,
    Error,
    Modifier,
    Variable,
    Udvt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInfo {
    pub kind: SymbolKind,
    pub name: String,
    pub range: TextRange,
    pub selection_range: TextRange,
    pub children: Vec<SymbolInfo>,
}

pub fn syntax_outline(parse: &Parse) -> Vec<SymbolInfo> {
    parse.with_session(|| collect_symbols(parse.tree()))
}

fn collect_symbols(tree: &SyntaxTree) -> Vec<SymbolInfo> {
    tree.items.iter().filter_map(symbol_from_item).collect()
}

fn symbol_from_item(item: &ast::Item<'static>) -> Option<SymbolInfo> {
    match &item.kind {
        ast::ItemKind::Contract(contract) => {
            let children = contract.body.iter().filter_map(symbol_from_item).collect();
            Some(SymbolInfo {
                kind: SymbolKind::Contract,
                name: contract.name.to_string(),
                range: span_to_text_range(item.span),
                selection_range: span_to_text_range(contract.name.span),
                children,
            })
        }
        ast::ItemKind::Function(function) => Some(SymbolInfo {
            kind: SymbolKind::Function,
            name: function
                .header
                .name
                .map(|name| name.to_string())
                .unwrap_or_else(|| function.kind.to_str().to_string()),
            range: span_to_text_range(item.span),
            selection_range: span_to_text_range(
                function
                    .header
                    .name
                    .map(|name| name.span)
                    .unwrap_or(function.header.span),
            ),
            children: Vec::new(),
        }),
        ast::ItemKind::Struct(item_struct) => Some(SymbolInfo {
            kind: SymbolKind::Struct,
            name: item_struct.name.to_string(),
            range: span_to_text_range(item.span),
            selection_range: span_to_text_range(item_struct.name.span),
            children: Vec::new(),
        }),
        _ => None,
    }
}

fn span_to_text_range(span: ast::Span) -> TextRange {
    let range = span.to_u32_range();
    TextRange::new(TextSize::from(range.start), TextSize::from(range.end))
}
