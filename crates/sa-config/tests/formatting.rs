use foundry_config::fmt::{DocCommentStyle, FormatterConfig, IndentStyle, QuoteStyle};
use sa_config::formatter_config;
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};

#[test]
fn formatter_config_tracks_foundry_settings() {
    let root = NormalizedPath::new("/workspace");
    let profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root);

    let formatter = FormatterConfig {
        line_length: 88,
        tab_width: 2,
        style: IndentStyle::Space,
        quote_style: QuoteStyle::Single,
        docs_style: DocCommentStyle::Block,
        ..Default::default()
    };

    let config = sa_config::ResolvedFoundryConfig::new(workspace, profile)
        .with_formatter_config(formatter.clone());
    let mapped = formatter_config(&config);

    assert_eq!(mapped.line_length, 88);
    assert_eq!(mapped.tab_width, 2);
    assert_eq!(mapped.style, IndentStyle::Space);
    assert_eq!(mapped.quote_style, QuoteStyle::Single);
    assert_eq!(mapped.docs_style, DocCommentStyle::Block);
}
