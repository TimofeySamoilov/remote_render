use bevy::prelude::*;
use bevy_tokio_tasks::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::key_board_control_server::{KeyBoardControl, KeyBoardControlServer};
use remote_render::{ControlRequest, FrameDispatch};
use std::sync::{Arc, Mutex};
use std::{error::Error, pin::Pin, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};

use bevy::prelude::Circle;
use bevy::render::view::screenshot::ScreenshotManager;
use bevy::sprite::MaterialMesh2dBundle;
use bevy::window::{PrimaryWindow, Window, WindowMode};

// this is bevy render with grpc-server
// uses special plugin: bevy_tokio_tasks
// which can allow using (tokio async + bevy app)

// server in bevy plugin system
pub mod remote_render {
    tonic::include_proto!("remote_render");
}
#[derive(Debug)]
pub struct RenderService {
    frame_receiver: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    client_connected: Arc<Mutex<bool>>,
}
impl RenderService {
    pub fn new(
        receiver: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
        client_connected: Arc<Mutex<bool>>,
    ) -> Self {
        RenderService {
            frame_receiver: receiver,
            client_connected: client_connected,
        }
    }
}
pub struct KeyBoardButtons {}
impl KeyBoardButtons {
    pub fn new() -> Self {
        KeyBoardButtons {}
    }
}

#[tonic::async_trait]
impl KeyBoardControl for KeyBoardButtons {
    async fn say_keyboard(
        &self,
        request: Request<ControlRequest>,
    ) -> Result<Response<ControlRequest>, Status> {
        let message = request.into_inner().message;
        println!("Received from client: {:?}", message);

        Ok(Response::new(ControlRequest {
            message: "OK".to_string(),
        }))
    }
}

#[tonic::async_trait]
impl Greeter for RenderService {
    type SayHelloStream = Pin<Box<dyn Stream<Item = Result<FrameDispatch, Status>> + Send>>;
    async fn say_hello(
        &self,
        request: Request<ControlRequest>,
    ) -> Result<Response<Self::SayHelloStream>, Status> {
        println!("{:?}", request.into_inner().message);
        let mut client_connected = self.client_connected.lock().unwrap();
        *client_connected = true;
        let (tx, rx) = mpsc::channel(128);
        let receiver_clone = self.frame_receiver.clone();
        tokio::spawn(async move {
            // communication with client
            loop {
                let screen: Vec<u8>;
                let data = receiver_clone.lock().unwrap().try_recv();

                match data {
                    Ok(frame) => {
                        screen = frame;
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(1)).await; // Try again soon
                        continue;
                    }
                }

                match tx
                    .send(Result::<_, Status>::Ok(FrameDispatch {
                        message: screen.clone(),
                    }))
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        println!("frame skipped, {:?}", e);
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
async fn start_server(
    receiver: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    client_connected: Arc<Mutex<bool>>,
) -> Result<(), Box<dyn Error>> {
    let greeter_addr = "[::1]:50051".parse()?;
    let keyboard_addr = "[::1]:50052".parse()?;

    let greeter = RenderService::new(receiver, client_connected);
    let keyboard = KeyBoardButtons::new();

    tokio::spawn(async move {
        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .serve(greeter_addr)
            .await
            .expect("Greeter server error");
    });

    Server::builder()
        .add_service(KeyBoardControlServer::new(keyboard))
        .serve(keyboard_addr)
        .await?;

    Ok(())
}

// system, which uses start_server()
fn server_system(runtime: ResMut<TokioTasksRuntime>, communication: ResMut<Communication>) {
    let receiver_clone = communication.receiver.clone();
    let client_connected = communication.client_connected.clone();
    runtime.spawn_background_task(|_ctx| async move {
        match start_server(receiver_clone, client_connected).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error with starting server: {:?}", e);
            }
        }
    });
}

fn main() {
    std::env::set_var("WGPU_BACKEND", "vulkan");
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Remote Render".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default())
        .add_systems(Startup, server_system)
        .add_systems(Startup, window_setup)
        .add_systems(Startup, scene_setup)
        .add_systems(Update, scene_update)
        .add_systems(PostUpdate, take_screenshot_system)
        .insert_resource(Communication::default())
        .run();
}

fn take_screenshot_system(
    mut screenshot_manager: ResMut<ScreenshotManager>,
    window_query: Query<(Entity, &Window), With<PrimaryWindow>>,
    communication: ResMut<Communication>,
) {
    let window_entity = window_query.single().0;
    let sender_clone = communication.sender.clone();
    let client_connected = communication.client_connected.clone();
    if *client_connected.lock().unwrap() {
        match screenshot_manager.take_screenshot(window_entity, move |image: Image| {
            let pixels = image.data;
            match sender_clone.try_send(pixels) {
                Ok(_) => {}
                Err(e) => {
                    println!("{:?}", e)
                }
            }
        }) {
            Ok(_) => {}
            Err(e) => {
                println!("Failed to create a screenshot: {:?}", e)
            }
        }
    }
}

// Bevy scene part
const SCREEN_WIDTH: f32 = 900.0;
const SCREEN_HEIGHT: f32 = 900.0;
const BALL_RADIUS: f32 = 50.0;
static mut BALL_SPEED: f32 = 120.0;

#[derive(Component)]
struct Ball;

#[derive(Resource)]
struct Communication {
    receiver: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    sender: mpsc::Sender<Vec<u8>>,
    client_connected: Arc<Mutex<bool>>,
}
impl Default for Communication {
    fn default() -> Self {
        let (sender_rx, receiver_rx): (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) =
            mpsc::channel(10000);
        Communication {
            receiver: Arc::new(Mutex::new(receiver_rx)),
            sender: sender_rx,
            client_connected: Arc::new(Mutex::new(false)),
        }
    }
}

fn window_setup(mut window: Query<&mut Window>) {
    let mut window = window.single_mut();
    window.resolution.set(SCREEN_WIDTH, SCREEN_HEIGHT);
    window.mode = WindowMode::Windowed;
}

fn scene_setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: meshes.add(Circle::new(BALL_RADIUS)).into(),
            material: materials.add(ColorMaterial::from(Color::WHITE)),
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)),
            ..default()
        },
        Ball,
    ));
}

fn scene_update(time: Res<Time>, mut query: Query<&mut Transform, With<Ball>>) {
    for mut transform in query.iter_mut() {
        let delta_time = time.delta_seconds();
        let move_amount = unsafe { BALL_SPEED } * delta_time;

        // moving
        transform.translation.x += move_amount;

        if transform.translation.x + BALL_RADIUS > SCREEN_WIDTH / 2.0 {
            transform.translation.x = SCREEN_WIDTH / 2.0 - BALL_RADIUS;
            unsafe {
                BALL_SPEED *= -1.0;
            }
        }

        if transform.translation.x - BALL_RADIUS < -SCREEN_WIDTH / 2.0 {
            transform.translation.x = -SCREEN_WIDTH / 2.0 + BALL_RADIUS;
            unsafe {
                BALL_SPEED *= -1.0;
            }
        }
    }
}
