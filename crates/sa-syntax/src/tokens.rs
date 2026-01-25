use sa_span::{TextRange, TextSize, is_ident_byte};
use solar_ast::token::{CommentKind, TokenKind};
use solar_interface::Session;
use solar_parse::Lexer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    pub is_doc: bool,
    pub text: String,
}

impl Comment {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            CommentKind::Line => "line",
            CommentKind::Block => "block",
        }
    }
}

pub fn collect_comments(text: &str) -> Vec<Comment> {
    let session = Session::builder()
        .with_silent_emitter(None)
        .single_threaded()
        .build();
    session.enter_sequential(|| {
        Lexer::new(&session, text)
            .filter_map(|token| match token.kind {
                TokenKind::Comment(is_doc, kind, symbol) => Some(Comment {
                    kind,
                    is_doc,
                    text: symbol.as_str().trim().to_string(),
                }),
                _ => None,
            })
            .collect()
    })
}

pub struct IdentRangeCollector {
    session: Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedIdentRange {
    pub range: TextRange,
    pub qualifier_start: TextSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedName {
    pub name: String,
    pub start: TextSize,
}

impl Default for IdentRangeCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentRangeCollector {
    pub fn new() -> Self {
        Self {
            session: Session::builder()
                .with_silent_emitter(None)
                .single_threaded()
                .build(),
        }
    }

    pub fn qualified_name_at_offset(
        &self,
        text: &str,
        offset: TextSize,
    ) -> Option<(Option<QualifiedName>, String)> {
        let idx = normalize_ident_offset(text, offset)?;
        self.session.enter_sequential(|| {
            let mut chain: Vec<(String, TextSize)> = Vec::new();
            let mut prev_was_dot = false;
            for token in Lexer::new(&self.session, text) {
                if token.is_comment_or_doc() {
                    continue;
                }
                let lo = token.span.lo().to_usize();
                let hi = token.span.hi().to_usize();
                match token.kind {
                    TokenKind::Ident(symbol) => {
                        let name = symbol.as_str().to_string();
                        let start = TextSize::from(lo as u32);
                        if prev_was_dot {
                            chain.push((name.clone(), start));
                        } else {
                            chain.clear();
                            chain.push((name.clone(), start));
                        }
                        prev_was_dot = false;

                        if lo <= idx && idx < hi {
                            let qualifier = if chain.len() > 1 {
                                let mut qualifier_name = String::new();
                                for (idx, (part, _)) in
                                    chain.iter().take(chain.len() - 1).enumerate()
                                {
                                    if idx > 0 {
                                        qualifier_name.push('.');
                                    }
                                    qualifier_name.push_str(part);
                                }
                                Some(QualifiedName {
                                    name: qualifier_name,
                                    start: chain[0].1,
                                })
                            } else {
                                None
                            };
                            return Some((qualifier, name));
                        }
                    }
                    TokenKind::Dot => {
                        if prev_was_dot {
                            chain.clear();
                        }
                        prev_was_dot = true;
                    }
                    _ => {
                        chain.clear();
                        prev_was_dot = false;
                    }
                }
            }
            None
        })
    }

    pub fn ident_range_at_offset(&self, text: &str, offset: TextSize) -> Option<TextRange> {
        let idx = normalize_ident_offset(text, offset)?;
        self.session.enter_sequential(|| {
            for token in Lexer::new(&self.session, text) {
                if token.is_comment_or_doc() {
                    continue;
                }
                if let TokenKind::Ident(_) = token.kind {
                    let lo = token.span.lo().to_usize();
                    let hi = token.span.hi().to_usize();
                    if lo <= idx && idx < hi {
                        let start = TextSize::from(lo as u32);
                        let end = TextSize::from(hi as u32);
                        return Some(TextRange::new(start, end));
                    }
                }
            }
            None
        })
    }

    pub fn collect(&self, text: &str, name: &str) -> Vec<TextRange> {
        if name.is_empty() {
            return Vec::new();
        }

        self.session.enter_sequential(|| {
            let mut ranges = Vec::new();
            for token in Lexer::new(&self.session, text) {
                if token.is_comment_or_doc() {
                    continue;
                }
                if let TokenKind::Ident(symbol) = token.kind
                    && symbol.as_str() == name
                {
                    let start = TextSize::from(token.span.lo().to_usize() as u32);
                    let end = TextSize::from(token.span.hi().to_usize() as u32);
                    ranges.push(TextRange::new(start, end));
                }
            }
            ranges
        })
    }

    pub fn collect_dot_qualified_ranges(&self, text: &str) -> Vec<TextRange> {
        self.session.enter_sequential(|| {
            let mut ranges = Vec::new();
            let mut prev_was_dot = false;
            for token in Lexer::new(&self.session, text) {
                if token.is_comment_or_doc() {
                    continue;
                }
                match token.kind {
                    TokenKind::Dot => {
                        prev_was_dot = true;
                    }
                    TokenKind::Ident(_) => {
                        if prev_was_dot {
                            let start = TextSize::from(token.span.lo().to_usize() as u32);
                            let end = TextSize::from(token.span.hi().to_usize() as u32);
                            ranges.push(TextRange::new(start, end));
                        }
                        prev_was_dot = false;
                    }
                    _ => {
                        prev_was_dot = false;
                    }
                }
            }
            ranges
        })
    }

    pub fn collect_qualified(
        &self,
        text: &str,
        qualifier: &str,
        name: &str,
    ) -> Vec<QualifiedIdentRange> {
        if qualifier.is_empty() || name.is_empty() {
            return Vec::new();
        }

        let qualifier_parts: Vec<&str> = qualifier.split('.').collect();
        if qualifier_parts.iter().any(|part| part.is_empty()) {
            return Vec::new();
        }
        let expected_chain_len = qualifier_parts.len() + 1;

        self.session.enter_sequential(|| {
            let mut ranges = Vec::new();
            let mut chain: Vec<(String, TextSize)> = Vec::new();
            let mut prev_was_dot = false;
            for token in Lexer::new(&self.session, text) {
                if token.is_comment_or_doc() {
                    continue;
                }
                let lo = token.span.lo().to_usize();
                let hi = token.span.hi().to_usize();
                match token.kind {
                    TokenKind::Ident(symbol) => {
                        let ident = symbol.as_str();
                        let start = TextSize::from(lo as u32);
                        if prev_was_dot {
                            chain.push((ident.to_string(), start));
                        } else {
                            chain.clear();
                            chain.push((ident.to_string(), start));
                        }
                        prev_was_dot = false;

                        if ident == name && chain.len() == expected_chain_len {
                            let mut is_match = true;
                            for (segment, part) in chain
                                .iter()
                                .take(qualifier_parts.len())
                                .zip(qualifier_parts.iter())
                            {
                                let (segment_name, _) = segment;
                                if segment_name.as_str() != *part {
                                    is_match = false;
                                    break;
                                }
                            }
                            if is_match {
                                ranges.push(QualifiedIdentRange {
                                    range: TextRange::new(
                                        TextSize::from(lo as u32),
                                        TextSize::from(hi as u32),
                                    ),
                                    qualifier_start: chain[0].1,
                                });
                            }
                        }
                    }
                    TokenKind::Dot => {
                        if prev_was_dot {
                            chain.clear();
                        }
                        prev_was_dot = true;
                    }
                    _ => {
                        chain.clear();
                        prev_was_dot = false;
                    }
                }
            }
            ranges
        })
    }
}

pub fn collect_ident_ranges(text: &str, name: &str) -> Vec<TextRange> {
    IdentRangeCollector::new().collect(text, name)
}

pub fn ident_range_at_offset(text: &str, offset: TextSize) -> Option<TextRange> {
    IdentRangeCollector::new().ident_range_at_offset(text, offset)
}

pub fn collect_qualified_ident_ranges(
    text: &str,
    qualifier: &str,
    name: &str,
) -> Vec<QualifiedIdentRange> {
    IdentRangeCollector::new().collect_qualified(text, qualifier, name)
}

fn normalize_ident_offset(text: &str, offset: TextSize) -> Option<usize> {
    if text.is_empty() {
        return None;
    }

    let mut idx: usize = offset.into();
    let bytes = text.as_bytes();
    if idx >= bytes.len() {
        idx = bytes.len() - 1;
    }

    if !is_ident_byte(bytes[idx]) {
        if idx == 0 || !is_ident_byte(bytes[idx - 1]) {
            return None;
        }
        idx -= 1;
    }

    Some(idx)
}
