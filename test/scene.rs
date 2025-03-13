use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy::asset::Asset;
use bevy::reflect::TypeUuid;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(MaterialPlugin::<NormalVisualizerMaterial>::default())
        .add_systems(Startup, setup)
        .run();
}

#[derive(AsBindGroup, TypePath, Debug, Clone, Asset, TypeUuid, Default)]
#[uuid = "f690fdae-d598-45a3-825e-d9cb6e2500e0"]
pub struct NormalVisualizerMaterial {}

impl Material for NormalVisualizerMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path("shaders/normals.wgsl".into())
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut normal_materials: ResMut<Assets<NormalVisualizerMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>, // Нужно добавить ResMut<Assets<StandardMaterial>>
) {
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(2.0, 2.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    commands.spawn(PointLightBundle {
        point_light: PointLight {
            intensity: 1500.0,
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0),
        ..default()
    });

    commands.spawn(PbrBundle {
        mesh: meshes.add(Mesh::from(shape::Cube { size: 1.0 })), // Использовать shape::Cube
        material: materials.add(StandardMaterial { // Создаём StandardMaterial
            base_color: Color::WHITE,
            ..default()
        }),
        transform: Transform::from_xyz(0.0, 0.5, 0.0),
        ..default()
    });
}

