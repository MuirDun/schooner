//! Asset-owned manual reload tooling.

use crate::action::Actions;
use crate::debug::egui::{self, Context};
use crate::debug::DebugPanels;
use crate::ecs::{Res, ResMut, World};
use crate::input::KeyCode;
use crate::plugin::Plugin;
use crate::render::{ForwardPipeline, MeshRegistry, RenderContext, TextureRegistry};
use crate::symbol::{Symbol, sym};
use crate::{App, Stage};

const RELOAD_ACTION: &str = "debug.assets.reload";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReloadSummary {
    pub meshes: usize,
    pub textures: usize,
    pub failures: u32,
}

#[derive(Debug)]
pub struct AssetDebugState {
    reload: Symbol,
    pub last_reload: Option<ReloadSummary>,
}

fn reload_assets(
    actions: Res<Actions>,
    mut state: ResMut<AssetDebugState>,
    ctx: Res<RenderContext>,
    mut meshes: ResMut<MeshRegistry>,
    mut textures: ResMut<TextureRegistry>,
    mut pipeline: ResMut<ForwardPipeline>,
) {
    if !actions.just_pressed(state.reload) {
        return;
    }

    let mesh_report = meshes.reload_all(ctx.device());
    let texture_report = textures.reload_all(ctx.device(), ctx.queue());
    for handle in &texture_report.reloaded {
        pipeline.invalidate_material_bind_group(*handle);
    }

    let summary = ReloadSummary {
        meshes: mesh_report.reloaded.len(),
        textures: texture_report.reloaded.len(),
        failures: mesh_report.failed + texture_report.failed,
    };
    state.last_reload = Some(summary);
    log::info!(
        "asset reload: {} mesh(es), {} texture(s) reloaded; {} failure(s)",
        summary.meshes,
        summary.textures,
        summary.failures,
    );
}

fn asset_panel(world: &mut World, ctx: &Context) {
    let last_reload = world
        .resource::<AssetDebugState>()
        .and_then(|state| state.last_reload);
    egui::Window::new("Assets")
        .default_open(false)
        .show(ctx, |ui| {
            ui.label("F5 reloads disk-backed meshes and textures");
            match last_reload {
                Some(summary) => {
                    ui.monospace(format!("Meshes:   {}", summary.meshes));
                    ui.monospace(format!("Textures: {}", summary.textures));
                    ui.monospace(format!("Failures: {}", summary.failures));
                }
                None => {
                    ui.label("No reload requested yet");
                }
            }
        });
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AssetDebugPlugin;

impl Plugin for AssetDebugPlugin {
    fn build(&self, app: App) -> App {
        let mut app = app
            .insert_resource(AssetDebugState {
                reload: sym(RELOAD_ACTION),
                last_reload: None,
            })
            .bind_key(RELOAD_ACTION, KeyCode::F5)
            .add_system(Stage::Update, reload_assets);
        if let Some(panels) = app.world_mut().resource_mut::<DebugPanels>() {
            panels.register("assets", asset_panel);
        }
        app
    }
}
