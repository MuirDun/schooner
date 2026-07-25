//! Typed application composition.
//!
//! Plugins are ordinary statically linked Rust values. They compose an
//! [`App`] at process startup; they are not dynamically loaded libraries.

use crate::App;

/// A typed unit of application composition.
///
/// Taking and returning [`App`] matches the engine's existing builder API, so
/// plugins can compose resources, systems, bindings, events, and other plugins
/// without a parallel in-place registration surface.
pub trait Plugin {
    fn build(&self, app: App) -> App;
}

/// The standard engine-side debug tool set.
///
/// This group is available only when the engine is built with `dev-tools`.
/// The group composes the core plus owner-side diagnostics, asset, and renderer
/// plugins. The core does not depend back on this convenience composition or
/// any of its target-subsystem members.
#[cfg(feature = "dev-tools")]
#[derive(Debug, Default, Clone, Copy)]
pub struct EngineDebugPlugins;

#[cfg(feature = "dev-tools")]
impl Plugin for EngineDebugPlugins {
    fn build(&self, app: App) -> App {
        app.add_plugin(crate::debug::DebugCorePlugin)
            .add_plugin(crate::diagnostics::DiagnosticsDebugPlugin)
            .add_plugin(crate::asset::AssetDebugPlugin)
            .add_plugin(crate::render::RenderDebugPlugin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Marker;
    struct MarkerPlugin;

    impl Plugin for MarkerPlugin {
        fn build(&self, app: App) -> App {
            app.insert_resource(Marker)
        }
    }

    #[test]
    fn add_plugin_composes_through_the_app_builder() {
        let mut app = App::new().add_plugin(MarkerPlugin);

        assert!(app.world_mut().resource::<Marker>().is_some());
    }
}
