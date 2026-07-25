//! Renderer-owned debug controls.

use crate::action::Actions;
use crate::debug::egui::{self, Context};
use crate::debug::DebugPanels;
use crate::ecs::{Res, ResMut, World};
use crate::input::KeyCode;
use crate::plugin::Plugin;
use crate::render::PcfKernel;
use crate::symbol::{Symbol, sym};
use crate::{App, Stage};

const CYCLE_PCF_ACTION: &str = "debug.render.pcf";

impl PcfKernel {
    fn cycle(self) -> Self {
        match self {
            PcfKernel::Single => PcfKernel::Soft3x3,
            PcfKernel::Soft3x3 => PcfKernel::Wide5x5,
            PcfKernel::Wide5x5 => PcfKernel::Single,
        }
    }
}

#[derive(Debug)]
struct RenderDebugActions {
    cycle_pcf: Symbol,
}

fn cycle_render_controls(
    actions: Res<Actions>,
    ids: Res<RenderDebugActions>,
    mut kernel: ResMut<PcfKernel>,
) {
    if actions.just_pressed(ids.cycle_pcf) {
        *kernel = kernel.cycle();
    }
}

fn render_panel(world: &mut World, ctx: &Context) {
    let kernel = world
        .resource::<PcfKernel>()
        .copied();
    egui::Window::new("Renderer")
        .default_open(false)
        .show(ctx, |ui| {
            ui.label("F1 cycles the shadow PCF selection");
            match kernel {
                Some(kernel) => ui.monospace(format!("PCF: {kernel:?}")),
                None => ui.label("PCF debug state unavailable"),
            };
            ui.small("This is the renderer's authoritative production resource");
        });
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderDebugPlugin;

impl Plugin for RenderDebugPlugin {
    fn build(&self, app: App) -> App {
        let mut app = app
            .insert_resource(RenderDebugActions {
                cycle_pcf: sym(CYCLE_PCF_ACTION),
            })
            .bind_key(CYCLE_PCF_ACTION, KeyCode::F1)
            .add_system(Stage::Update, cycle_render_controls);
        if let Some(panels) = app.world_mut().resource_mut::<DebugPanels>() {
            panels.register("renderer", render_panel);
        }
        app
    }
}
