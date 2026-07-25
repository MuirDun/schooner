//! Kinesis-owned render debugging.
//!
//! These controls cycle the production render resources directly. The engine
//! owns the effects and their neutral defaults; this game owns which looks are
//! useful to compare and the asset used to exercise the overlay slot.

use glam::Vec3;
use schooner_engine::debug::egui::{self, Context};
use schooner_engine::ecs::{Res, ResMut, World};
use schooner_engine::{
    Actions, App, Bloom, ColorGrade, DebugPanels, Fog, KeyCode, OverlayBlend, Plugin, PostOverlay,
    Stage, Symbol, Vignette, sym,
};

use crate::scene::assets::{Assets, TextureAsset};

const CYCLE_GRADE_ACTION: &str = "debug.kinesis.grade";
const CYCLE_VIGNETTE_ACTION: &str = "debug.kinesis.vignette";
const CYCLE_FOG_ACTION: &str = "debug.kinesis.fog";
const CYCLE_OVERLAY_ACTION: &str = "debug.kinesis.overlay";
const CYCLE_BLOOM_ACTION: &str = "debug.kinesis.bloom";

/// A denser version of the chamber atmosphere used only as the far end of
/// Kinesis's fog comparison. It stays here because it is an art choice, not an
/// engine default.
const DENSE_CHAMBER_FOG: Fog = Fog {
    color: Vec3::new(0.02, 0.03, 0.05),
    base_height: 0.0,
    density: 0.08,
    falloff: 0.3,
    scattering: 0.25,
};

#[derive(Debug)]
struct KinesisRenderDebugActions {
    grade: Symbol,
    vignette: Symbol,
    fog: Symbol,
    overlay: Symbol,
    bloom: Symbol,
}

fn next_grade(current: ColorGrade) -> ColorGrade {
    if current == ColorGrade::DEFAULT {
        ColorGrade::CHAMBER_WHITE
    } else if current == ColorGrade::CHAMBER_WHITE {
        ColorGrade::CAGE_WARM
    } else if current == ColorGrade::CAGE_WARM {
        ColorGrade::SERVICE_RED
    } else {
        ColorGrade::DEFAULT
    }
}

fn next_vignette(current: Vignette) -> Vignette {
    if current == Vignette::DEFAULT {
        Vignette::CINEMATIC
    } else if current == Vignette::CINEMATIC {
        Vignette::OPPRESSIVE
    } else {
        Vignette::DEFAULT
    }
}

fn next_fog(current: Fog) -> Fog {
    if current == Fog::DEFAULT {
        Fog::SCARSE_CHAMBER
    } else if current == Fog::SCARSE_CHAMBER {
        DENSE_CHAMBER_FOG
    } else {
        Fog::DEFAULT
    }
}

fn next_bloom(current: Bloom) -> Bloom {
    if current == Bloom::OFF {
        Bloom::FAINT
    } else if current == Bloom::FAINT {
        Bloom::ERA_GLOW
    } else if current == Bloom::ERA_GLOW {
        Bloom::EVERYTHING_GLOWS
    } else {
        Bloom::OFF
    }
}

fn cycle_grade(
    actions: Res<Actions>,
    ids: Res<KinesisRenderDebugActions>,
    mut grade: ResMut<ColorGrade>,
) {
    if actions.just_pressed(ids.grade) {
        *grade = next_grade(*grade);
    }
}

fn cycle_vignette(
    actions: Res<Actions>,
    ids: Res<KinesisRenderDebugActions>,
    mut vignette: ResMut<Vignette>,
) {
    if actions.just_pressed(ids.vignette) {
        *vignette = next_vignette(*vignette);
    }
}

fn cycle_fog(actions: Res<Actions>, ids: Res<KinesisRenderDebugActions>, mut fog: ResMut<Fog>) {
    if actions.just_pressed(ids.fog) {
        *fog = next_fog(*fog);
    }
}

fn cycle_overlay(
    actions: Res<Actions>,
    ids: Res<KinesisRenderDebugActions>,
    assets: Res<Assets>,
    mut overlay: ResMut<PostOverlay>,
) {
    if !actions.just_pressed(ids.overlay) {
        return;
    }

    if overlay.intensity <= 0.0 {
        if overlay.texture.is_none() {
            overlay.texture = assets.try_texture(TextureAsset::MetalCube);
        }
        overlay.intensity = 0.8;
        overlay.blend = OverlayBlend::AlphaBlend;
    } else {
        match overlay.blend {
            OverlayBlend::AlphaBlend => {
                overlay.intensity = 1.0;
                overlay.blend = OverlayBlend::Multiply;
            }
            OverlayBlend::Multiply => {
                overlay.intensity = 0.4;
                overlay.blend = OverlayBlend::Additive;
            }
            OverlayBlend::Additive => {
                overlay.intensity = 0.0;
                overlay.blend = OverlayBlend::AlphaBlend;
            }
        }
    }
}

fn cycle_bloom(
    actions: Res<Actions>,
    ids: Res<KinesisRenderDebugActions>,
    mut bloom: ResMut<Bloom>,
) {
    if actions.just_pressed(ids.bloom) {
        *bloom = next_bloom(*bloom);
    }
}

fn grade_name(value: ColorGrade) -> &'static str {
    if value == ColorGrade::DEFAULT {
        "Default"
    } else if value == ColorGrade::CHAMBER_WHITE {
        "Chamber white"
    } else if value == ColorGrade::CAGE_WARM {
        "Cage warm"
    } else if value == ColorGrade::SERVICE_RED {
        "Service red"
    } else {
        "Custom"
    }
}

fn vignette_name(value: Vignette) -> &'static str {
    if value == Vignette::DEFAULT {
        "Off"
    } else if value == Vignette::CINEMATIC {
        "Cinematic"
    } else if value == Vignette::OPPRESSIVE {
        "Oppressive"
    } else {
        "Custom"
    }
}

fn fog_name(value: Fog) -> &'static str {
    if value == Fog::DEFAULT {
        "Off"
    } else if value == Fog::SCARSE_CHAMBER {
        "Sparse chamber"
    } else if value == DENSE_CHAMBER_FOG {
        "Dense chamber"
    } else {
        "Custom"
    }
}

fn bloom_name(value: Bloom) -> &'static str {
    if value == Bloom::OFF {
        "Off"
    } else if value == Bloom::FAINT {
        "Faint"
    } else if value == Bloom::ERA_GLOW {
        "Era glow"
    } else if value == Bloom::EVERYTHING_GLOWS {
        "Everything glows"
    } else {
        "Custom"
    }
}

fn overlay_name(value: &PostOverlay) -> &'static str {
    if value.intensity <= 0.0 {
        "Off"
    } else {
        match value.blend {
            OverlayBlend::AlphaBlend => "Alpha",
            OverlayBlend::Multiply => "Multiply",
            OverlayBlend::Additive => "Additive",
        }
    }
}

fn kinesis_render_panel(world: &mut World, ctx: &Context) {
    let grade = world.resource::<ColorGrade>().copied();
    let vignette = world.resource::<Vignette>().copied();
    let fog = world.resource::<Fog>().copied();
    let bloom = world.resource::<Bloom>().copied();
    let overlay = world
        .resource::<PostOverlay>()
        .map(|value| (overlay_name(value), value.texture.is_some()));

    egui::Window::new("Kinesis Render")
        .default_open(false)
        .show(ctx, |ui| {
            ui.monospace(format!(
                "F2  Grade: {}",
                grade.map(grade_name).unwrap_or("Unavailable")
            ));
            ui.monospace(format!(
                "F3  Vignette: {}",
                vignette.map(vignette_name).unwrap_or("Unavailable")
            ));
            ui.monospace(format!(
                "F4  Fog: {}",
                fog.map(fog_name).unwrap_or("Unavailable")
            ));
            ui.monospace(format!(
                "F6  Overlay: {}",
                overlay.map(|value| value.0).unwrap_or("Unavailable")
            ));
            ui.monospace(format!(
                "F7  Bloom: {}",
                bloom.map(bloom_name).unwrap_or("Unavailable")
            ));
            if overlay.is_some_and(|value| !value.1) {
                ui.small("Overlay test texture is not resident; using renderer fallback");
            }
            ui.small("These are the live production render resources");
        });
}

/// Installs the render comparisons that belong specifically to Kinesis.
#[derive(Debug, Default, Clone, Copy)]
pub struct KinesisRenderDebugPlugin;

impl Plugin for KinesisRenderDebugPlugin {
    fn build(&self, app: App) -> App {
        let mut app = app
            .insert_resource(KinesisRenderDebugActions {
                grade: sym(CYCLE_GRADE_ACTION),
                vignette: sym(CYCLE_VIGNETTE_ACTION),
                fog: sym(CYCLE_FOG_ACTION),
                overlay: sym(CYCLE_OVERLAY_ACTION),
                bloom: sym(CYCLE_BLOOM_ACTION),
            })
            .bind_key(CYCLE_GRADE_ACTION, KeyCode::F2)
            .bind_key(CYCLE_VIGNETTE_ACTION, KeyCode::F3)
            .bind_key(CYCLE_FOG_ACTION, KeyCode::F4)
            .bind_key(CYCLE_OVERLAY_ACTION, KeyCode::F6)
            .bind_key(CYCLE_BLOOM_ACTION, KeyCode::F7)
            .add_system(Stage::Update, cycle_grade)
            .add_system(Stage::Update, cycle_vignette)
            .add_system(Stage::Update, cycle_fog)
            .add_system(Stage::Update, cycle_overlay)
            .add_system(Stage::Update, cycle_bloom);

        if let Some(panels) = app.world_mut().resource_mut::<DebugPanels>() {
            panels.register("kinesis-render", kinesis_render_panel);
        }
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_cycle_returns_to_default() {
        let value = next_grade(ColorGrade::DEFAULT);
        let value = next_grade(value);
        let value = next_grade(value);
        assert_eq!(next_grade(value), ColorGrade::DEFAULT);
    }

    #[test]
    fn fog_cycle_is_kinesis_owned() {
        assert_eq!(next_fog(Fog::DEFAULT), Fog::SCARSE_CHAMBER);
        assert_eq!(next_fog(Fog::SCARSE_CHAMBER), DENSE_CHAMBER_FOG);
        assert_eq!(next_fog(DENSE_CHAMBER_FOG), Fog::DEFAULT);
    }

    #[test]
    fn bloom_cycle_reaches_both_extremes() {
        assert_eq!(next_bloom(Bloom::ERA_GLOW), Bloom::EVERYTHING_GLOWS);
        assert_eq!(next_bloom(Bloom::EVERYTHING_GLOWS), Bloom::OFF);
    }
}
