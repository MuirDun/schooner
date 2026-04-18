use winit::dpi::PhysicalSize;

#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub size: PhysicalSize<u32>,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            size: PhysicalSize::new(width, height),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self::new("Schooner", 1280, 720)
    }
}
