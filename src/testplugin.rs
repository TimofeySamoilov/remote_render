use bevy::{prelude::*, transform::commands};
use tokio::runtime::Runtime;
pub struct MyPlugin { runtime: Option<Runtime>,}
impl MyPlugin {
    pub fn init(&mut self) {
        println!("Init from plugin!");
        self.runtime = Some(Runtime::new().expect("Runtime creation fail!"));
    }
    pub fn send(&self) {
        if let Some(runtime) = &self.runtime {
            runtime.spawn(async {
                while (1 == 1) {
                    println!("hiii");
                }
            });
        }
    }
}

impl Plugin for MyPlugin {
    fn build(&self, app: &mut App) {
        
    }
}

fn main() {
    let mut async_plugin = MyPlugin { runtime: None };
    async_plugin.init();
    async_plugin.send();
    
    std::env::set_var("WGPU_BACKEND", "vulkan");

    App::new().add_plugins(async_plugin).run();
}