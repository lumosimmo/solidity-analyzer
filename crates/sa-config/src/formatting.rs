use foundry_config::fmt::FormatterConfig;

use crate::ResolvedFoundryConfig;

pub fn formatter_config(config: &ResolvedFoundryConfig) -> FormatterConfig {
    config.formatter_config().clone()
}
