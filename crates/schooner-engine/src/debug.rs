//! Typed host for Rust-side debug tooling.
//!
//! The core owns composition-neutral concerns only: overlay visibility, the
//! runtime panel registry, and the Render-stage UI build. Diagnostics, assets,
//! rendering, physics, and games contribute their own typed plugins.

pub use egui;
use egui::Context;

#[cfg(feature = "dev-tools")]
use crate::action::Actions;
#[cfg(feature = "dev-tools")]
use crate::ecs::{Res, ResMut, exclusive};
use crate::ecs::World;
#[cfg(feature = "dev-tools")]
use crate::input::KeyCode;
#[cfg(feature = "dev-tools")]
use crate::plugin::Plugin;
use crate::render::DebugOverlay;
#[cfg(feature = "dev-tools")]
use crate::symbol::{Symbol, sym};
#[cfg(feature = "dev-tools")]
use crate::{App, Stage};

#[cfg(feature = "dev-tools")]
const TOGGLE_OVERLAY_ACTION: &str = "debug.overlay.toggle";

/// Debug-host state. Target-subsystem settings do not belong here.
#[derive(Debug)]
pub struct DebugState {
    /// Master visibility — F12 through `debug.overlay.toggle` toggles it.
    pub overlay_visible: bool,
}

impl Default for DebugState {
    fn default() -> Self {
        Self {
            overlay_visible: true,
        }
    }
}

#[cfg(feature = "dev-tools")]
#[derive(Debug)]
struct DebugCoreActions {
    toggle_overlay: Symbol,
}

/// A debug-panel contribution with typed access to the live ECS world.
///
/// Panel functions build their own egui window or surface. They keep mutable
/// state in typed world resources rather than capturing an erased value bag.
pub type DebugPanel = fn(&mut World, &Context);

#[derive(Clone, Copy)]
struct RegisteredPanel {
    name: &'static str,
    build: DebugPanel,
}

/// Runtime registry of statically typed debug-panel contributions.
///
/// Registration order is execution order. Re-registering a name replaces its
/// callback in place, preserving order and avoiding duplicate panels.
#[derive(Default)]
pub struct DebugPanels {
    panels: Vec<RegisteredPanel>,
}

impl DebugPanels {
    pub fn register(&mut self, name: &'static str, build: DebugPanel) {
        if let Some(panel) = self.panels.iter_mut().find(|panel| panel.name == name) {
            panel.build = build;
        } else {
            self.panels.push(RegisteredPanel { name, build });
        }
    }

    pub fn len(&self) -> usize {
        self.panels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    fn build(&self, world: &mut World, ctx: &Context) {
        for panel in &self.panels {
            (panel.build)(world, ctx);
        }
    }
}

/// Exclusive Render-stage system that prepares the egui frame.
///
/// The renderer owns only pass encoding. UI discovery and typed world access
/// happen here, before `render_frame`. The overlay and registry are temporarily
/// removed so callbacks can access the rest of the world without aliasing.
pub fn build_debug_overlay(world: &mut World) {
    let Some(mut overlay) = world.remove_resource::<DebugOverlay>() else {
        return;
    };
    let Some(panels) = world.remove_resource::<DebugPanels>() else {
        world.insert_resource(overlay);
        return;
    };

    let visible = world
        .resource::<DebugState>()
        .is_some_and(|debug| debug.overlay_visible);
    overlay.run(visible, |ctx| panels.build(world, ctx));

    world.insert_resource(panels);
    world.insert_resource(overlay);
}

#[cfg(feature = "dev-tools")]
fn toggle_debug_overlay(
    actions: Res<Actions>,
    ids: Res<DebugCoreActions>,
    mut debug: ResMut<DebugState>,
) {
    if actions.just_pressed(ids.toggle_overlay) {
        debug.overlay_visible = !debug.overlay_visible;
    }
}

/// Minimal debug host. It deliberately installs no target-subsystem tools.
#[cfg(feature = "dev-tools")]
#[derive(Debug, Default, Clone, Copy)]
pub struct DebugCorePlugin;

#[cfg(feature = "dev-tools")]
impl Plugin for DebugCorePlugin {
    fn build(&self, app: App) -> App {
        let mut app = app.enable_debug_core();
        if app.world_mut().contains_resource::<DebugState>() {
            return app;
        }
        app
            .insert_resource(DebugState::default())
            .insert_resource(DebugPanels::default())
            .insert_resource(DebugCoreActions {
                toggle_overlay: sym(TOGGLE_OVERLAY_ACTION),
            })
            .bind_key(TOGGLE_OVERLAY_ACTION, KeyCode::F12)
            .add_system(Stage::Update, toggle_debug_overlay)
            .add_system(Stage::Render, exclusive(build_debug_overlay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Debug, PartialEq)]
    struct Calls(Vec<&'static str>);

    fn first(world: &mut World, _: &Context) {
        world.resource_mut::<Calls>().unwrap().0.push("first");
    }

    fn second(world: &mut World, _: &Context) {
        world.resource_mut::<Calls>().unwrap().0.push("second");
    }

    #[test]
    fn panels_build_in_registration_order() {
        let mut panels = DebugPanels::default();
        panels.register("first", first);
        panels.register("second", second);
        let mut world = World::new();
        world.insert_resource(Calls::default());

        panels.build(&mut world, &Context::default());

        assert_eq!(
            world.resource::<Calls>(),
            Some(&Calls(vec!["first", "second"]))
        );
    }

    #[test]
    fn registering_the_same_name_replaces_in_place() {
        let mut panels = DebugPanels::default();
        panels.register("panel", first);
        panels.register("panel", second);
        let mut world = World::new();
        world.insert_resource(Calls::default());

        panels.build(&mut world, &Context::default());

        assert_eq!(panels.len(), 1);
        assert_eq!(world.resource::<Calls>(), Some(&Calls(vec!["second"])));
    }
}
