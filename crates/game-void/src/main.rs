use schooner_engine::logging::{self, LogConfig};
use schooner_engine::{App, WindowConfig};

fn main() -> anyhow::Result<()> {
    logging::init(LogConfig::default())?;

    App::new()
        .with_window_config(WindowConfig::new("Schooner — The Void", 1280, 720))
        .with_fps_logging()
        .with_input_logging()
        .run()?;
    Ok(())
}
