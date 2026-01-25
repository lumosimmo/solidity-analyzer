use crate::{Parse, SyntaxTree};
use solar_ast::{Item, ItemFunction, ItemKind};

pub fn top_level_items(tree: &SyntaxTree) -> impl Iterator<Item = &Item<'static>> + '_ {
    tree.items.iter()
}

pub fn contract_names(parse: &Parse) -> Vec<String> {
    parse.with_session(|| {
        top_level_items(parse.tree())
            .filter_map(|item| match &item.kind {
                ItemKind::Contract(contract) => Some(contract.name.to_string()),
                _ => None,
            })
            .collect()
    })
}

pub fn function_signatures(parse: &Parse) -> Vec<String> {
    parse.with_session(|| {
        let mut signatures = Vec::new();
        collect_functions(top_level_items(parse.tree()), &mut signatures);
        signatures
    })
}

fn collect_functions<'a>(items: impl Iterator<Item = &'a Item<'static>>, out: &mut Vec<String>) {
    for item in items {
        match &item.kind {
            ItemKind::Function(function) => out.push(format_signature(function)),
            ItemKind::Contract(contract) => collect_functions(contract.body.iter(), out),
            _ => {}
        }
    }
}

fn format_signature(function: &ItemFunction<'_>) -> String {
    let param_count = function.header.parameters.vars.len();
    let mut signature = function.kind.to_str().to_string();
    if let Some(name) = function.header.name {
        signature.push(' ');
        signature.push_str(&name.to_string());
    }
    signature.push('(');
    signature.push_str(&param_count.to_string());
    signature.push(')');
    signature
}
