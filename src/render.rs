use bevy::prelude::*;
pub struct myplugin;

#[derive(Component)]
struct Position {
    x: f32,
    y: f32,
}
fn print_position_system(query: Query<&Position>) {
    for position in &query {
        println!("position: {} {}", position.x, position.y);
    }
}
struct Entity(u64);


impl Plugin for myplugin {
    fn build(&self, app: &mut App) {
        println!("hi");
    }
}

fn main() {
    
    std::env::set_var("WGPU_BACKEND", "vulkan");

    App::new().add_systems(Update, print_position_system)
        .add_plugins(DefaultPlugins)
        .run();
}