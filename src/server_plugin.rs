use bevy::prelude::*;
use bevy_tokio_tasks::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::{HelloReply, HelloRequest};
use std::{error::Error, pin::Pin, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};
use std::sync::{Arc, Mutex};

use bevy::prelude::Circle;
use bevy::render::{view::screenshot::ScreenshotManager};
use bevy::sprite::MaterialMesh2dBundle;
use bevy::window::{Window, PrimaryWindow, WindowMode};
// this is bevy render with grpc-server
// uses special plugin: bevy_tokio_tasks
// which can allow using (tokio async + bevy app)

// server in bevy plugin system
pub mod remote_render {
    tonic::include_proto!("remote_render");
}

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    type SayHelloStream = Pin<Box<dyn Stream<Item = Result<HelloReply, Status>> + Send>>;

    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<Self::SayHelloStream>, Status> {
        println!("{:?}", request.into_inner().message);

        let (tx, rx) = mpsc::channel(128);

        tokio::spawn(async move {
            let mut a: u8 = 0;
            let mut screen = vec![0; 500 * 500 * 4];
            // communication with client
            while let Ok(_) = tx
                .send(Result::<_, Status>::Ok(HelloReply {
                    message: screen.clone(),
                }))
                .await
            {
                tokio::time::sleep(Duration::from_millis(64)).await;
                a = (a + 1) % 3;
                let save = set_color(a);
                for x in 0..500 {
                    for y in 0..500 {
                        let index = (y * 500 + x) * 4;
                        if x == y || x + y == 500 - 1 {
                            (screen[index], screen[index + 1], screen[index + 2]) = (255, 255, 255);
                            continue;
                        }
                        (screen[index], screen[index + 1], screen[index + 2]) = save;
                        screen[index + 3] = 128;
                    }
                }
            }
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::SayHelloStream
        ))
    }
}

// function, which starts sever
async fn start_server() -> Result<(), Box<dyn Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter {};
    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;
    Ok(())
}

// system, which uses start_server()
fn server_system(runtime: ResMut<TokioTasksRuntime>) {
    runtime.spawn_background_task(|_ctx| async move {
        match start_server().await {
            Ok(_) => {
                println!("Server started...");
            }
            Err(e) => {
                eprintln!("Error with starting server: {:?}", e);
            }
        }
    });
}

fn main() {
    std::env::set_var("WGPU_BACKEND", "vulkan");
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default())
        .add_systems(Startup, server_system)
        .add_systems(Startup, window_setup)
        .add_systems(Startup, scene_setup)
        .add_systems(Update, update)
        .insert_resource(Communication::default())
        .run();
}

// this function return different colors
fn set_color(a: u8) -> (u8, u8, u8) {
    match a {
        0 => (255, 0, 0),
        1 => (0, 255, 0),
        _ => (0, 0, 255),
    }
}

const SCREEN_WIDTH: f32 = 900.0;
const SCREEN_HEIGHT: f32 = 900.0;
const BALL_RADIUS: f32 = 50.0;
static mut BALL_SPEED: f32 = 120.0;

#[derive(Component)]
struct Ball;

#[derive(Resource)]
struct Communication {
    receiver: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    sender: Arc<Mutex<mpsc::Sender<Vec<u8>>>>
}
impl Default for Communication {
    fn default() -> Self {
        let (sender_rx, receiver_rx): (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) = mpsc::channel(1);
        Communication {receiver: Arc::new(Mutex::new(receiver_rx)), sender: Arc::new(Mutex::new(sender_rx))}
    }
}

fn window_setup(mut window: Query<&mut Window>) {
    let mut window = window.single_mut();
    window.resolution.set(SCREEN_WIDTH, SCREEN_HEIGHT);
    window.mode = WindowMode::Windowed;
}

fn scene_setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<ColorMaterial>>) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn((MaterialMesh2dBundle {
        mesh: meshes.add(Circle::new(BALL_RADIUS)).into(),
        material: materials.add(ColorMaterial::from(Color::WHITE)),
        transform: Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)),
        ..default()
    }, Ball));
}

fn update(
    time: Res<Time>,
    mut query: Query<&mut Transform, With<Ball>>,
    mut screenshot_manager: ResMut<ScreenshotManager>,
    window_query: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut communication: ResMut<Communication>,
) {
    let window_entity = window_query.single().0;
    for mut transform in query.iter_mut() {
        let delta_time = time.delta_seconds();
        let move_amount = unsafe { BALL_SPEED } * delta_time;

        // moving
        transform.translation.x += move_amount;

        if transform.translation.x + BALL_RADIUS > SCREEN_WIDTH / 2.0 {
             transform.translation.x = SCREEN_WIDTH / 2.0 - BALL_RADIUS;
             unsafe { BALL_SPEED *= -1.0; }

        }

        if transform.translation.x - BALL_RADIUS < -SCREEN_WIDTH / 2.0 {
            transform.translation.x = -SCREEN_WIDTH / 2.0 + BALL_RADIUS;
            unsafe { BALL_SPEED *= -1.0; }
        }
    }
    let sender_clone = communication.sender.clone();
    match screenshot_manager.take_screenshot(window_entity, move |image: Image| {
        let width = image.width();
        let height = image.height();
        let pixels = image.data;

        if let Ok(mut sender) = sender_clone.lock() {
            let _ = sender.try_send(pixels);
        }

    }) {
        Ok(_) => { println!("Ok Screenshot") }
        Err(e) => {println!("Failed to create a screenshot: {:?}", e)}
    }
}
