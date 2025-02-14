use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::{
        camera::RenderTarget,
        render_asset::{RenderAssetUsages, RenderAssets},
        render_graph::{self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d,
            ImageCopyBuffer, ImageDataLayout, Maintain, MapMode, TextureDimension, TextureFormat,
            TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::{BevyDefault, TextureFormatPixelInfo},
        Extract, Render, RenderApp, RenderSet,
    },
};
use crossbeam_channel::{Receiver, Sender};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use bevy::prelude::Circle;
use bevy_tokio_tasks::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::key_board_control_server::{KeyBoardControl, KeyBoardControlServer};
use remote_render::{ControlRequest, FrameDispatch};
use std::sync::Mutex;
use std::{error::Error, pin::Pin};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};

use lz4::{EncoderBuilder, Decoder};
use std::io::{Write, Read};


pub mod remote_render {
    tonic::include_proto!("remote_render");
}

use tracing_subscriber::{filter::EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use tracing::{span, Level};
use tracing_subscriber::fmt;

/// This will receive asynchronously any data sent from the render world
#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<Vec<u8>>);

/// This will send asynchronously any data to the main world
#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

// Parameters of resulting image
struct AppConfig {
    width: u32,
    height: u32,
    single_image: bool,
}

fn main() {
    let filter = match std::env::var("RUST_LOG") {
        Ok(val) => {
            println!("RUST_LOG: {}", val);
            EnvFilter::try_new(val).expect("failed to parse RUST_LOG")
        }
        Err(_) => {
            println!("RUST_LOG is not set, using default filter");
            EnvFilter::try_new("remote_render=trace").expect("failed to parse filter")
        }
    };
    tracing_subscriber::registry().with(fmt::layer()).with(filter).init();

    let config = AppConfig {
        width: 900,
        height: 900,
        single_image: false,
    };
    // setup frame capture
    App::new()
        .insert_resource(SceneController::new(
            config.width,
            config.height,
            config.single_image,
        ))
        .insert_resource(ClearColor(Color::srgb_u8(0, 0, 0)))
        .insert_resource(Communication::default())
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default())
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                // Do not create a window on startup.
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    close_when_requested: false,
                })
                .disable::<bevy::log::LogPlugin>(),
        )
        .add_plugins(ImageCopyPlugin)
        // headless frame capture
        .add_plugins(CaptureFramePlugin)
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 1000.0,
        )))
        .init_resource::<SceneController>()
        .add_systems(Startup, setup)
        .add_systems(Startup, server_system)
        .add_systems(Update, scene_update)
        .add_systems(Update, movement)
        .add_systems(PostUpdate, update)
        .add_systems(Update, extract_normals_system.after(update))
        .run();
}

enum ChannelMessage {
    Pixels(Vec<u8>),
    PixelsAndFrame(Vec<u8>, u32)
}


// server part
#[derive(Resource)]
struct Communication {
    receiver: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
    sender: mpsc::Sender<ChannelMessage>,
    client_connected: Arc<Mutex<bool>>,
    frame_number: Arc<Mutex<u32>>,
    lz4_compression: bool,
    pressed_key: Arc<Mutex<u8>>
}
impl Default for Communication {
    fn default() -> Self {
        let (sender_rx, receiver_rx): (mpsc::Sender<ChannelMessage>, mpsc::Receiver<ChannelMessage>) =
            mpsc::channel(10000);
        Communication {
            receiver: Arc::new(Mutex::new(receiver_rx)),
            sender: sender_rx,
            client_connected: Arc::new(Mutex::new(false)),
            frame_number: Arc::new(Mutex::new(0)),
            lz4_compression: true,
            pressed_key: Arc::new(Mutex::new(0u8))
        }
    }
}


#[derive(Debug)]
struct RenderService {
    frame_receiver: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
    client_connected: Arc<Mutex<bool>>,
}
impl RenderService {
    pub fn new(
        receiver: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
        client_connected: Arc<Mutex<bool>>,
    ) -> Self {
        RenderService {
            frame_receiver: receiver,
            client_connected: client_connected,
        }
    }
}
pub struct KeyBoardButtons {
    pressed_key: Arc<Mutex<u8>>
}
impl KeyBoardButtons {
    pub fn new(key: Arc<Mutex<u8>>) -> Self {
        KeyBoardButtons {
            pressed_key: key
        }
    }
}

#[tonic::async_trait]
impl KeyBoardControl for KeyBoardButtons {
    async fn say_keyboard(
        &self,
        request: Request<ControlRequest>,
    ) -> Result<Response<ControlRequest>, Status> {
        let message = request.into_inner().message;
        let mut key = self.pressed_key.lock().unwrap();
        *key = compare_key(message);
        Ok(Response::new(ControlRequest {
            message: "OK".to_string(),
        }))
    }
}
fn compare_key(s: String) -> u8 {
    if s == "KeyA" { return 1; }
    if s == "KeyW" { return 2; }
    if s == "KeyD" { return 3; }
    if s == "KeyS" { return 4; }
    if s == "KeyQ" { return 5; }
    if s == "KeyE" { return 6; }
    if s == "Space" { return 7; }
    if s == "ControlLeft" { return 8; }
    return 0;
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
                let mut frame: u32 = 0;
                match data {
                    Ok(channel_message) => {
                        match channel_message {
                            ChannelMessage::PixelsAndFrame(pixels, id) => {
                                screen = pixels;
                                frame = id;
                            }
                            _ => {
                                tokio::time::sleep(Duration::from_millis(1)).await; // Try again soon
                                continue;
                            }
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(1)).await; // Try again soon
                        continue;
                    }
                }
                let span = span!(Level::TRACE, "frame", n = frame);
                let _enter = span.enter();
                match tx.try_send(Result::<_, Status>::Ok(FrameDispatch {
                    message: screen.clone(),
                    frame: frame
                })) {
                    Ok(_) => {
                        trace!("frame sent to client");
                    }
                    Err(e) => {
                        trace!("frame was skipped! {:?}", e);
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
    receiver: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
    client_connected: Arc<Mutex<bool>>,
    pressed_key: Arc<Mutex<u8>>
) -> Result<(), Box<dyn Error>> {
    let greeter_addr = "[::1]:50051".parse()?;
    let greeter = RenderService::new(receiver, client_connected);
    let keyboard = KeyBoardButtons::new(pressed_key);

    tokio::spawn(async move {
        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .add_service(KeyBoardControlServer::new(keyboard))
            .serve(greeter_addr)
            .await
            .expect("Greeter server error");
    });
    Ok(())
}

// system, which uses start_server()
fn server_system(runtime: ResMut<TokioTasksRuntime>, communication: ResMut<Communication>) {
    let receiver_clone = communication.receiver.clone();
    let client_connected = communication.client_connected.clone();
    let pressed_key = communication.pressed_key.clone();
    runtime.spawn_background_task(|_ctx| async move {
        match start_server(receiver_clone, client_connected, pressed_key).await {
            Ok(_) => {
                info!("Server started...")
            }
            Err(e) => {
                error!("Error with starting server: {:?}", e);
            }
        }
    });
}
// end of server part
/// Capture image settings and state
#[derive(Debug, Default, Resource)]
struct SceneController {
    state: SceneState,
    name: String,
    width: u32,
    height: u32,
    single_image: bool,
}

impl SceneController {
    pub fn new(width: u32, height: u32, single_image: bool) -> SceneController {
        SceneController {
            state: SceneState::BuildScene,
            name: String::from(""),
            width,
            height,
            single_image,
        }
    }
}

/// Capture image state
#[derive(Debug, Default)]
enum SceneState {
    #[default]
    // State before any rendering
    BuildScene,
    // Rendering state, stores the number of frames remaining before saving the image
    Render(u32),
}

#[derive(Component)]
struct Cube;

#[derive(Component)]
struct CameraSettings {
    position: Vec3,
    look_at: Vec3,
    up: Vec3,
    speed: f32 // Camera movement speed
}

impl Default for CameraSettings {
    fn default() -> Self {
        CameraSettings {
            position: Vec3::new(-2.5, 4.5, 9.0),
            look_at: Vec3::ZERO,
            up: Vec3::Y,
            speed: 5.0
        }
    }
}

#[derive(Component)]
struct MainCamera; // Marker component for the main camera

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    render_device: Res<RenderDevice>,
) {
    let render_target = setup_render_target(
        &mut commands,
        &mut images,
        &render_device,
        &mut scene_controller,
        // pre_roll_frames should be big enough for full scene render,
        // but the bigger it is, the longer example will run.
        // To visualize stages of scene rendering change this param to 0
        // and change AppConfig::single_image to false in main
        // Stages are:
        // 1. Transparent image
        // 2. Few black box images
        // 3. Fully rendered scene images
        // Exact number depends on device speed, device load and scene size
        40,
        "main_scene".into(),
    );

    // Scene example for non black box picture
    // circular base
    commands.spawn(PbrBundle {
        mesh: meshes.add(Circle::new(4.0)),
        material: materials.add(Color::WHITE),
        transform: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ..default()
    });
    // cube
    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(124, 144, 255)),
            transform: Transform {
                translation: Vec3::new(0.0, 0.5, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), // Поворот на 45 градусов вокруг оси Y
                scale: Vec3::new(1.0, 1.0, 1.0),
            },
            ..default()
        },
        Cube, // Добавление маркера компонента Cube
    ));
    // light
    commands.spawn(PointLightBundle {
        point_light: PointLight {
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0),
        ..default()
    });
    let camera_settings = CameraSettings::default();
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_translation(camera_settings.position)
                .looking_at(camera_settings.look_at, camera_settings.up),
            tonemapping: Tonemapping::None,
            camera: Camera {
                target: render_target,
                ..default()
            },
            ..default()
        },
        camera_settings,
        MainCamera,
    ));
}
fn movement(
    mut cam_query: Query<(&mut Transform, &CameraSettings), With<MainCamera>>,
    communication: Res<Communication>,
    time: Res<Time>
) {
    let pressed_key = *communication.pressed_key.lock().unwrap();
    
    if let Ok((mut camera, settings)) = cam_query.get_single_mut() {
        let speed = settings.speed * time.delta_seconds();
        match pressed_key {
            1 => {
                let left = camera.left();
                camera.translation += left * speed;
            }
            2 => {
                let forward = camera.forward();
                camera.translation += forward * speed;
            }
            3 => {
                let right = camera.right();
                camera.translation += right * speed;
            }
            4 => {
                let backward = camera.back();
                camera.translation += backward * speed;
            }
            5 => {
                camera.rotate_y(speed / 5.0);
            }
            6 => {
                camera.rotate_y(-speed / 5.0);
            }
            7 => {
                let up = camera.up();
                camera.translation += up * speed;
            }
            8 => {
                let down = camera.down();
                camera.translation += down * speed;
            }
            _ => {}
        }
    }
}
fn scene_update(
    mut cube_query: Query<&mut Transform, With<Cube>>,
    time: Res<Time>,
) {
    // Move the cube 
    for mut transform in cube_query.iter_mut() {
        let move_distance = 3.0;
        let move_speed = 0.5;
        let cycle_time = 2.0;
        let relative_time = time.elapsed_seconds() * move_speed % cycle_time;

        if relative_time < cycle_time / 2.0 {
            transform.translation.x = move_distance * (relative_time - cycle_time / 4.0);
        } else {
            transform.translation.x =
                move_distance * (cycle_time / 4.0 - (relative_time - cycle_time / 2.0));
        }
    }
}

/// Plugin for Render world part of work
pub struct ImageCopyPlugin;
impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();

        let render_app = app
            .insert_resource(MainWorldReceiver(r))
            .sub_app_mut(RenderApp);

        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(ImageCopy, ImageCopyDriver);
        graph.add_node_edge(bevy::render::graph::CameraDriverLabel, ImageCopy);

        render_app
            .insert_resource(RenderWorldSender(s))
            // Make ImageCopiers accessible in RenderWorld system and plugin
            .add_systems(ExtractSchedule, image_copy_extract)
            // Receives image data from buffer to channel
            // so we need to run it after the render graph is done
            .add_systems(Render, receive_image_from_buffer.after(RenderSet::Render));
    }
}

/// Setups render target and cpu image for saving, changes scene state into render mode
fn setup_render_target(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    render_device: &Res<RenderDevice>,
    scene_controller: &mut ResMut<SceneController>,
    pre_roll_frames: u32,
    scene_name: String,
) -> RenderTarget {
    let size = Extent3d {
        width: scene_controller.width,
        height: scene_controller.height,
        ..Default::default()
    };

    // This is the texture that will be rendered to.
    let mut render_target_image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::bevy_default(),
        RenderAssetUsages::default(),
    );
    render_target_image.texture_descriptor.usage |=
        TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    let render_target_image_handle = images.add(render_target_image);

    // This is the texture that will be copied to.
    let cpu_image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::bevy_default(),
        RenderAssetUsages::default(),
    );
    let cpu_image_handle = images.add(cpu_image);

    commands.spawn(ImageCopier::new(
        render_target_image_handle.clone(),
        size,
        render_device,
    ));

    commands.spawn(ImageToSave(cpu_image_handle));

    scene_controller.state = SceneState::Render(pre_roll_frames);
    scene_controller.name = scene_name;
    RenderTarget::Image(render_target_image_handle)
}

/// Setups image saver
pub struct CaptureFramePlugin;
impl Plugin for CaptureFramePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update);
    }
}

/// `ImageCopier` aggregator in `RenderWorld`
#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ImageCopiers(pub Vec<ImageCopier>);

/// Used by `ImageCopyDriver` for copying from render target to buffer
#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    src_image: Handle<Image>,
}

impl ImageCopier {
    pub fn new(
        src_image: Handle<Image>,
        size: Extent3d,
        render_device: &RenderDevice,
    ) -> ImageCopier {
        let padded_bytes_per_row =
            RenderDevice::align_copy_bytes_per_row((size.width) as usize) * 4;

        let cpu_buffer = render_device.create_buffer(&BufferDescriptor {
            label: None,
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        ImageCopier {
            buffer: cpu_buffer,
            src_image,
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// Extracting `ImageCopier`s into render world, because `ImageCopyDriver` accesses them
fn image_copy_extract(mut commands: Commands, image_copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(
        image_copiers.iter().cloned().collect::<Vec<ImageCopier>>(),
    ));
}

/// `RenderGraph` label for `ImageCopyDriver`
#[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
struct ImageCopy;

/// `RenderGraph` node
#[derive(Default)]
struct ImageCopyDriver;

// Copies image content from render target to buffer
impl render_graph::Node for ImageCopyDriver {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let image_copiers = world.get_resource::<ImageCopiers>().unwrap();
        let gpu_images = world
            .get_resource::<RenderAssets<bevy::render::texture::GpuImage>>()
            .unwrap();

        for image_copier in image_copiers.iter() {
            if !image_copier.enabled() {
                continue;
            }

            let src_image = gpu_images.get(&image_copier.src_image).unwrap();

            let mut encoder = render_context
                .render_device()
                .create_command_encoder(&CommandEncoderDescriptor::default());

            let block_dimensions = src_image.texture_format.block_dimensions();
            let block_size = src_image.texture_format.block_copy_size(None).unwrap();

            let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
                (src_image.size.x as usize / block_dimensions.0 as usize) * block_size as usize,
            );

            let texture_extent = Extent3d {
                width: src_image.size.x,
                height: src_image.size.y,
                depth_or_array_layers: 1,
            };

            encoder.copy_texture_to_buffer(
                src_image.texture.as_image_copy(),
                ImageCopyBuffer {
                    buffer: &image_copier.buffer,
                    layout: ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            std::num::NonZeroU32::new(padded_bytes_per_row as u32)
                                .unwrap()
                                .into(),
                        ),
                        rows_per_image: None,
                    },
                },
                texture_extent,
            );

            let render_queue = world.get_resource::<RenderQueue>().unwrap();
            render_queue.submit(std::iter::once(encoder.finish()));
        }

        Ok(())
    }
}

/// runs in render world after Render stage to send image from buffer via channel (receiver is in main world)
fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for image_copier in image_copiers.0.iter() {
        if !image_copier.enabled() {
            continue;
        }

        // Finally time to get our data back from the gpu.
        // First we get a buffer slice which represents a chunk of the buffer (which we
        // can't access yet).
        // We want the whole thing so use unbounded range.
        let buffer_slice = image_copier.buffer.slice(..);

        let (s, r) = crossbeam_channel::bounded(1);

        // Maps the buffer so it can be read on the cpu
        buffer_slice.map_async(MapMode::Read, move |r| match r {
            // This will execute once the gpu is ready, so after the call to poll()
            Ok(r) => s.send(r).expect("Failed to send map update"),
            Err(err) => panic!("Failed to map buffer {err}"),
        });

        // In order for the mapping to be completed, one of three things must happen.
        // One of those can be calling `Device::poll`. This isn't necessary on the web as devices
        // are polled automatically but natively, we need to make sure this happens manually.
        // `Maintain::Wait` will cause the thread to wait on native but not on WebGpu.

        // This blocks until the gpu is done executing everything
        render_device.poll(Maintain::wait()).panic_on_timeout();

        // This blocks until the buffer is mapped
        r.recv().expect("Failed to receive the map_async message");

        // This could fail on app exit, if Main world clears resources (including receiver) while Render world still renders
        let _ = sender.send(buffer_slice.get_mapped_range().to_vec());

        // We need to make sure all `BufferView`'s are dropped before we do what we're about
        // to do.
        // Unmap so that we can copy to the staging buffer in the next iteration.
        image_copier.buffer.unmap();
    }
}

/// CPU-side image for saving
#[derive(Component, Deref, DerefMut)]
struct ImageToSave(Handle<Image>);

// Takes from channel image content sent from render world and saves it to disk
fn update(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    mut app_exit_writer: EventWriter<AppExit>,
    communication: ResMut<Communication>,
) {
    if let SceneState::Render(n) = scene_controller.state {
        if n < 1 {
            // We don't want to block the main world on this,
            // so we use try_recv which attempts to receive without blocking
            let mut image_data = Vec::new();
            while let Ok(data) = receiver.try_recv() {
                // image generation could be faster than saving to fs,
                // that's why use only last of them
                image_data = data;
            }
            if !image_data.is_empty() {
                for image in images_to_save.iter() {
                    // Fill correct data from channel to image
                    let img_bytes = images.get_mut(image.id()).unwrap();

                    // We need to ensure that this works regardless of the image dimensions
                    // If the image became wider when copying from the texture to the buffer,
                    // then the data is reduced to its original size when copying from the buffer to the image.
                    let row_bytes = img_bytes.width() as usize
                        * img_bytes.texture_descriptor.format.pixel_size();
                    let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);
                    if row_bytes == aligned_row_bytes {
                        img_bytes.data.clone_from(&image_data);
                    } else {
                        // shrink data to original image size
                        img_bytes.data = image_data
                            .chunks(aligned_row_bytes)
                            .take(img_bytes.height() as usize)
                            .flat_map(|row| &row[..row_bytes.min(row.len())])
                            .cloned()
                            .collect();
                    }

                    // Create RGBA Image Buffer
                    let img = match img_bytes.clone().try_into_dynamic() {
                        Ok(img) => img.to_rgba8(),
                        Err(e) => panic!("Failed to create image buffer {e:?}"),
                    };
                    let rgba_pixels: Vec<u8> = img.into_raw();

                    let client_connected = communication.client_connected.clone();
                    let sender_clone = communication.sender.clone();
                    if *client_connected.lock().unwrap() {

                        let frames = communication.frame_number.clone();
                        let mut frame_number = frames.lock().unwrap();
                        *frame_number = (*frame_number + 1) % 1000;

                        let frame = span!(Level::TRACE, "frame", n = *frame_number);
                        let _enter = frame.enter();
                        
                        trace!("Was received from main world");

                        let mut frame_to_send: Vec<u8> = vec![];
                        if communication.lz4_compression {
                            match encode_image_lz4(&rgba_pixels) {
                                Ok(value) => {
                                    frame_to_send = value;
                                },
                                Err(e) => {
                                    trace!("Error with lz4!, {:?}", e);
                                    return;
                                }
                            }
                        }
                        else {
                            frame_to_send = rgba_pixels;
                        }
                        match sender_clone.try_send(ChannelMessage::PixelsAndFrame(frame_to_send, *frame_number)) {
                            Ok(_) => {trace!("Was sended from update to server")}
                            Err(e) => {
                                println!("error with sending {:?}", e)
                            }
                        }
                    }

                }
                if scene_controller.single_image {
                    app_exit_writer.send(AppExit::Success);
                }
            }
        } else {
            // clears channel for skipped frames
            while receiver.try_recv().is_ok() {}
            scene_controller.state = SceneState::Render(n - 1);
        }
    }
}

fn extract_normals_system(
    query: Query<&Handle<Mesh>>,
    meshes: Res<Assets<Mesh>>,
) {
    let mut all_normals = Vec::new();
    for mesh_handle in query.iter() {
        if let Some(mesh) = meshes.get(mesh_handle) {

            if let Some(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
                if let Some(normals) = normals.as_float3() {
                    all_normals.extend(normals.iter().cloned());
                }
            }
        }
    }
    println!("{:?}", all_normals);
}

fn encode_image_lz4(image_data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut compressed_data = Vec::new(); // Create a Vec to hold the compressed data
    // Create the encoder, using the Vec as the writer.  This is KEY.
    let mut encoder = EncoderBuilder::new()
        .level(2)
        .build(&mut compressed_data)?;

    // Write the image data to the encoder. Now it works because 'encoder' implements Write.
    let _ = encoder.write_all(image_data)?;

    // Finish the encoder (no need for extra read_to_end). The compressed data is already in `compressed_data`.
    let _ = encoder.finish();
    Ok(compressed_data)
}