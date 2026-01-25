use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintFixKind {
    MixedCaseVariable,
    MixedCaseFunction,
    PascalCaseStruct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LintFix {
    pub code: &'static str,
    pub title: &'static str,
    pub kind: LintFixKind,
}

const LINT_FIXES: &[LintFix] = &[
    LintFix {
        code: "mixed-case-variable",
        title: "Convert to mixedCase",
        kind: LintFixKind::MixedCaseVariable,
    },
    LintFix {
        code: "mixed-case-function",
        title: "Convert to mixedCase",
        kind: LintFixKind::MixedCaseFunction,
    },
    LintFix {
        code: "pascal-case-struct",
        title: "Convert to PascalCase",
        kind: LintFixKind::PascalCaseStruct,
    },
];

static LINT_FIX_LOOKUP: OnceLock<HashMap<&'static str, &'static LintFix>> = OnceLock::new();

pub fn lint_fix(code: &str) -> Option<&'static LintFix> {
    let lookup = LINT_FIX_LOOKUP.get_or_init(|| {
        let mut lookup = HashMap::with_capacity(LINT_FIXES.len());
        for fix in LINT_FIXES {
            lookup.insert(fix.code, fix);
        }
        lookup
    });
    lookup.get(code).copied()
}

pub fn is_fixable_lint(code: &str) -> bool {
    lint_fix(code).is_some()
}
