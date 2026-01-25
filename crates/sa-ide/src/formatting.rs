use forge_fmt::{FormatterConfig, FormatterResult};
use sa_span::{TextRange, TextSize};

use crate::TextEdit;

pub fn format_edit(text: &str, config: &FormatterConfig) -> Option<TextEdit> {
    let formatted = match forge_fmt::format(text, config.clone()) {
        FormatterResult::Ok(formatted)
        | FormatterResult::OkWithDiagnostics(formatted, _)
        | FormatterResult::ErrRecovered(formatted, _) => formatted,
        FormatterResult::Err(_) => return None,
    };

    if formatted == text {
        return None;
    }

    let end = TextSize::try_from(text.len()).ok()?;
    Some(TextEdit {
        range: TextRange::new(TextSize::from(0), end),
        new_text: formatted,
    })
}
