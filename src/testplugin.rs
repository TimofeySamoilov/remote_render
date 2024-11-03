use bevy::{prelude::*, transform::commands};
pub struct MyPlugin;
impl MyPlugin {
    fn init(&self) {
        println!("Hello from plugin!");
    }
    fn send(&self) {
        println!("hello from send!");
    }
}

impl Plugin for MyPlugin {
    fn build(&self, app: &mut App) {
        self.init();
        self.send();
    }
}


fn main() {
    std::env::set_var("WGPU_BACKEND", "vulkan");

    App::new().add_plugins(MyPlugin).run();
}