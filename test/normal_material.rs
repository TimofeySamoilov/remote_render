use bevy::{
    asset::*,
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::*,
        render_resource::*,
        render_asset::RenderAssetPlugin,
        extract_resource::ExtractResource,
    },
};

#[derive(Resource, Clone, ExtractResource, Reflect, Default)]
struct NormalShader {
    handle: Handle<Shader>,
}

fn setup_normal_shader(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let normal_shader = NormalShader {
        handle: asset_server.load("shaders/normals.wgsl"),
    };

    commands.insert_resource(normal_shader);
}

fn replace_standard_material_shader<StandardMaterialPipeline>(
    shader: Res<NormalShader>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Заменяем шейдер для всех материалов на наш шейдер для нормалей
    for (_, material) in materials.iter_mut() {
        material.fragment_shader = Some(shader.handle.clone());
    }
}
