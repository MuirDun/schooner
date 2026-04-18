//! Logging setup for Schooner games.
//!
//! The `log` facade routes all records through one global logger, but
//! every record carries a target string (defaulting to the emitting
//! module's path). Engine records therefore start with `schooner_engine`
//! and game records start with the game crate's name — filter the two
//! independently via `RUST_LOG`:
//!
//! ```text
//! RUST_LOG=schooner_engine=debug,game_void=info,wgpu=warn
//! ```

use std::env;

use env_logger::Builder;
use log::LevelFilter;

/// Configuration for the global logger.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Level applied when no per-target directive matches.
    pub default_level: LevelFilter,
    /// `env_logger`-format filter string pre-applied before `RUST_LOG`.
    /// Useful for quieting chatty dependencies (wgpu, naga, winit).
    pub fallback_filter: Option<String>,
}

impl LogConfig {
    pub fn with_default_level(mut self, level: LevelFilter) -> Self {
        self.default_level = level;
        self
    }

    pub fn with_fallback_filter(mut self, filter: impl Into<String>) -> Self {
        self.fallback_filter = Some(filter.into());
        self
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            default_level: LevelFilter::Info,
            fallback_filter: Some(
                "wgpu_core=warn,wgpu_hal=warn,naga=warn,winit=warn".into(),
            ),
        }
    }
}

/// Install the global logger.
///
/// Precedence, highest to lowest: `RUST_LOG` env var → `fallback_filter`
/// → `default_level`. Call once from the game binary's `main`.
pub fn init(config: LogConfig) -> Result<(), log::SetLoggerError> {
    let mut builder = Builder::new();
    builder.filter_level(config.default_level);
    if let Some(fallback) = config.fallback_filter.as_deref() {
        builder.parse_filters(fallback);
    }
    if let Ok(rust_log) = env::var("RUST_LOG") {
        builder.parse_filters(&rust_log);
    }
    builder.try_init()
}
