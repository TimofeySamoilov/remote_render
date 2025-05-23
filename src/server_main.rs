mod server;
use crate::server::{Communication, server_system, ChannelMessage, CamerasMovement};
use std::sync::Mutex;
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

use lz4::EncoderBuilder;
use std::io::Write;

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
        .insert_resource(CamerasMovement::default())
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
        .add_systems(PostUpdate, update)
        .add_systems(Update, move_cameras)
        //.add_systems(Update, extract_normals_system.after(update))
        .run();
}

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

const NUM_CAMERAS: usize = 4;
#[derive(Resource, Clone)]
pub struct Cameras {
    cameras: Vec<Entity>,
    robots: Vec<Entity>,
    receivers: Arc<Mutex<Vec<Receiver<Vec<u8>>>>>,
    senders: Arc<Mutex<Vec<Sender<Vec<u8>>>>>,
}
impl Default for Cameras {
    fn default() -> Self {
        let mut cameras = Vec::new();
        let mut robots = Vec::new();
        let mut receivers = Arc::new(Mutex::new(Vec::new()));
        let mut senders = Arc::new(Mutex::new(Vec::new()));
        return Cameras {
            cameras: cameras,
            robots: robots,
            receivers: receivers,
            senders: senders,
        }
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>
) {
    let mut cameras = Vec::new();
    let mut robots = Vec::new();
    let mut receivers = Arc::new(Mutex::new(Vec::new()));
    let mut senders = Arc::new(Mutex::new(Vec::new()));

    for i in 0..4 {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let render_target = setup_render_target(
            &mut commands,
            &mut images,
            &render_device,
            &mut scene_controller,
            40,
            format!("main_scene_{}", i),
            i
        );

        let camera_entity = commands.spawn((Camera3dBundle {
            transform: Transform::from_translation([0.0, 1.0, 0.0].into()),
            camera: Camera {
                target: render_target,
                ..default()
            },
            ..default()
        },)).id();

        let robot_handle: Handle<Scene> = asset_server.load("models/robot.glb#Scene0");
        let robot_entity = commands.spawn(SceneBundle {
            scene: robot_handle.clone(),
            transform: Transform {
                translation: Vec3::new(0.0, 0.8, 2.0),  // Смещаем по X для каждого робота
                rotation: Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
                scale: Vec3::splat(0.5),
            },
            ..default()
        }).id();

        robots.push(robot_entity);
        cameras.push(camera_entity);
        receivers.lock().unwrap().push(receiver);
        senders.lock().unwrap().push(sender);
    }


    commands.insert_resource(Cameras {
        cameras,
        robots,
        receivers,
        senders,
    });

    let shelf_handle: Handle<Scene> = asset_server.load("models/shelf.glb#Scene0");
    commands.spawn(SceneBundle {
        scene: shelf_handle,
        transform: Transform { translation: Vec3::new(4.0, 0.7, -5.0), rotation: Quat::default(), scale: (Vec3 { x: (0.5), y: (0.5), z: (0.5) }) },
        ..default()
    });
    let shelf_handle: Handle<Scene> = asset_server.load("models/shelf.glb#Scene0");
    commands.spawn(SceneBundle {
        scene: shelf_handle,
        transform: Transform { translation: Vec3::new(4.0, 0.7, 5.0), rotation: Quat::default(), scale: (Vec3 { x: (0.5), y: (0.5), z: (0.5) }) },
        ..default()
    });
    let shelf_handle: Handle<Scene> = asset_server.load("models/shelf.glb#Scene0");
    commands.spawn(SceneBundle {
        scene: shelf_handle,
        transform: Transform { translation: Vec3::new(-4.0, 0.7, 0.0), rotation: Quat::from_rotation_y(140.0), scale: (Vec3 { x: (0.5), y: (0.5), z: (0.5) }) },
        ..default()
    });

    let box_handle: Handle<Scene> = asset_server.load("models/box.glb#Scene0");
    commands.spawn(SceneBundle {
        scene: box_handle,
        transform: Transform { translation: Vec3::new(3.5, 0.3, 3.5), rotation: Quat::default(), scale: (Vec3 { x: (0.5), y: (0.5), z: (0.5) }) },
        ..default()
    }); 

    // Walls
    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(143, 218, 163)),
            transform: Transform {
                translation: Vec3::new(-20.0, 4.5, 0.0),
                rotation: Quat::default(),
                scale: Vec3::new(1.0, 10.0, 80.0),
            },
            ..default()
        },
    ));

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(216, 192, 94)),
            transform: Transform {
                translation: Vec3::new(20.0, 4.5, 0.0),
                rotation: Quat::default(),
                scale: Vec3::new(1.0, 10.0, 80.0),
            },
            ..default()
        },
    ));

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(156, 183, 201)),
            transform: Transform {
                translation: Vec3::new(0.0, 4.5, 20.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                scale: Vec3::new(1.0, 10.0, 80.0),
            },
            ..default()
        },
    ));

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(79, 169, 202)),
            transform: Transform {
                translation: Vec3::new(0.0, 4.5, -20.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                scale: Vec3::new(1.0, 10.0, 80.0),
            },
            ..default()
        },
    ));


    // circular base
    commands.spawn(PbrBundle {
        mesh: meshes.add(Circle::new(40.0)),
        material: materials.add(Color::WHITE),
        transform: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ..default()
    });
    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            material: materials.add(Color::srgb_u8(104, 144, 45)),
            transform: Transform {
                translation: Vec3::new(2.0, 0.5, 2.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                scale: Vec3::new(1.0, 1.0, 1.0),
            },
            ..default()
        },
    ));
    commands.spawn((
        PbrBundle {
            // Заменяем Cuboid на Sphere с радиусом 0.5
            mesh: meshes.add(Sphere::new(0.5)), 
            material: materials.add(Color::srgb_u8(15, 100, 200)),
            transform: Transform {
                translation: Vec3::new(-2.0, 0.5, 5.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                scale: Vec3::new(1.0, 1.0, 1.0),
            },
            ..default()
        },
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
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 5000.0, // Освещенность в люксах
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(0.0, 10.0, 0.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });
}

fn extract_cameras(
    mut commands: Commands,
    cameras: Extract<Res<Cameras>>, // Используем Extract для явного указания источника
) {
    commands.insert_resource(cameras.clone());
}

/// Plugin for Render world part of work
pub struct ImageCopyPlugin;
impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {

        let (s, r) = crossbeam_channel::unbounded();

        let render_app = app
            .insert_resource(MainWorldReceiver(r))
            .sub_app_mut(RenderApp);

        //render_app.init_resource::<Cameras>();

        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(ImageCopy, ImageCopyDriver);
        graph.add_node_edge(bevy::render::graph::CameraDriverLabel, ImageCopy);
        render_app
            .insert_resource(RenderWorldSender(s))
            .add_systems(ExtractSchedule, extract_cameras)
            // Make ImageCopiers accessible in RenderWorld system and plugin
            .add_systems(ExtractSchedule, (
                extract_cameras,
                image_copy_extract
            ).chain())
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
    camera_id: usize,
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
        camera_id
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
    camera_index: usize
}

impl ImageCopier {
    pub fn new(
        src_image: Handle<Image>,
        size: Extent3d,
        render_device: &RenderDevice,
        camera_index: usize
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
            camera_index
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
    cameras: Res<Cameras>,
) {
    let senders_guard = cameras.senders.lock().unwrap();
    
    // Перебираем копировщики по индексам
    for (index, copier) in image_copiers.0.iter().enumerate() {
        if !copier.enabled() {
            continue;
        }

        // Проверяем наличие соответствующего sender
        let Some(sender) = senders_guard.get(index) else {
            error!("No sender for camera index {}", index);
            continue;
        };

        let buffer_slice = copier.buffer.slice(..);
        let (sync_s, sync_r) = crossbeam_channel::bounded(1);

        buffer_slice.map_async(MapMode::Read, move |result| {
            sync_s.send(result).unwrap_or_else(|_| error!("Sync channel closed"));
        });

        render_device.poll(Maintain::wait()).panic_on_timeout();

        match sync_r.recv() {
            Ok(Ok(())) => {
                let data = buffer_slice.get_mapped_range().to_vec();
                if let Err(e) = sender.send(data) {
                    error!("Failed to send image for camera {}: {}", index, e);
                }
                else {
                    //println!("data sent to camera {:?}", index);
                }
            },
            Ok(Err(err)) => error!("Buffer mapping failed for camera {}: {}", index, err),
            Err(_) => error!("Sync channel error for camera {}", index),
        }

        copier.buffer.unmap();
    }
}

/// CPU-side image for saving
#[derive(Component, Deref, DerefMut)]
struct ImageToSave(Handle<Image>);

// Takes from channel image content sent from render world and saves it to disk
fn update(
    images_to_save: Query<&ImageToSave>,
    cameras: Res<Cameras>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    mut app_exit_writer: EventWriter<AppExit>,
    communication: ResMut<Communication>,
) {
    if let SceneState::Render(n) = scene_controller.state {
        if n < 1 {
            let receivers_guard = match cameras.receivers.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            // Собираем данные для всех камер
            let mut camera_frames = Vec::new();
            let num_cameras = receivers_guard.len();

            for (camera_index, receiver) in receivers_guard.iter().enumerate() {
                let mut image_data = Vec::new();
                
                // Читаем все данные для камеры
                while let Ok(data) = receiver.try_recv() {
                    image_data = data;
                }

                // Обрабатываем изображение только если есть данные
                if !image_data.is_empty() {
                    if let Some(image_to_save) = images_to_save.iter().nth(camera_index) {
                        let img_bytes = images.get_mut(image_to_save.id()).unwrap();

                        // Обработка формата изображения
                        let row_bytes = img_bytes.width() as usize
                            * img_bytes.texture_descriptor.format.pixel_size();
                        let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);

                        if row_bytes == aligned_row_bytes {
                            img_bytes.data.clone_from(&image_data);
                        } else {
                            img_bytes.data = image_data
                                .chunks(aligned_row_bytes)
                                .take(img_bytes.height() as usize)
                                .flat_map(|row| &row[..row_bytes.min(row.len())])
                                .cloned()
                                .collect();
                        }

                        // Конвертация в raw пиксели
                        let img = match img_bytes.clone().try_into_dynamic() {
                            Ok(img) => img.to_rgba8(),
                            Err(e) => panic!("Failed to create image buffer {e:?}"),
                        };

                        camera_frames.push(Some(img.into_raw()));
                    }
                } else {
                    camera_frames.push(None);
                }
            }

            // Отправляем данные клиентам
            let clients = communication.clients.clone();
            let clients_guard = clients.lock().unwrap();
            let mut frame_numbers_guard = communication.frame_numbers.lock().unwrap();

            for (client_idx, client_id) in clients_guard.iter().enumerate() {
                if client_idx >= communication.senders.len() {
                    break;
                }

                // Определяем камеру для клиента
                let camera_index = client_idx % camera_frames.len();
                
                // Получаем данные для этой камеры
                if let Some(rgba_pixels) = &camera_frames[camera_index] {
                    let frame_to_send = if communication.lz4_compression {
                        match encode_image_lz4(rgba_pixels) {
                            Ok(mut compressed) => {
                                let mut header = camera_index.to_be_bytes().to_vec();
                                header.append(&mut compressed);
                                header
                            }
                            Err(e) => {
                                trace!("LZ4 error: {:?}", e);
                                continue;
                            }
                        }
                    } else {
                        rgba_pixels.clone()
                    };

                    // Обновляем номер кадра
                    frame_numbers_guard[client_idx] = (frame_numbers_guard[client_idx] + 1) % 1000;

                    // Отправляем сообщение
                    match communication.senders[client_idx].try_send(
                        ChannelMessage::PixelsAndFrame(
                            frame_to_send,
                            frame_numbers_guard[client_idx],
                        )
                    ) {
                        Ok(_) => trace!(
                            "Sent frame {} from camera {} to client {}",
                            frame_numbers_guard[client_idx],
                            camera_index,
                            client_id
                        ),
                        Err(e) => error!("Send error to client {}: {:?}", client_id, e),
                    }
                }
            }

            if scene_controller.single_image {
                app_exit_writer.send(AppExit::Success);
            }
        } else {
            let mut receivers_guard = cameras.receivers.lock().unwrap();
            for receiver in receivers_guard.iter_mut() {
                while receiver.try_recv().is_ok() {}
            }
            scene_controller.state = SceneState::Render(n - 1);
        }
    }
}

/*fn extract_normals_system(
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
}*/

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

pub fn move_cameras(
    mut cameras_movement: ResMut<CamerasMovement>,
    cameras: ResMut<Cameras>,
    communication: ResMut<Communication>,
    mut transforms: Query<&mut Transform>,
) {
    let button_receiver = &mut cameras_movement.button_receiver;
    let clients = communication.clients.lock().unwrap();
    let cameras_entities = &cameras.cameras;

    while let Ok((id, button)) = button_receiver.try_recv() {
        if let Some(client_idx) = clients.iter().position(|&client_id| client_id == id) {
            let camera_entity = cameras_entities[client_idx];
            let robot_entity = cameras.robots[client_idx];

            // 1. Обрабатываем камеру
            let (new_cam_transform, camera_offset) = {
                if let Ok(mut cam_transform) = transforms.get_mut(camera_entity) {
                    let speed = 0.02;
                    let rotation_speed = 0.01;
                    
                    let old_rotation = cam_transform.rotation;
                    let old_position = cam_transform.translation;

                    // Двигаем камеру
                    let forward = old_rotation * Vec3::NEG_Z;
                    let right = old_rotation * Vec3::X;

                    match button.as_str() {
                        "A" => cam_transform.translation -= right * speed,
                        "D" => cam_transform.translation += right * speed,
                        "W" => cam_transform.translation += forward * speed,
                        "S" => cam_transform.translation -= forward * speed,
                        "Q" => cam_transform.rotate_y(rotation_speed),
                        "E" => cam_transform.rotate_y(-rotation_speed),
                        _ => (),
                    }

                    // Сохраняем новые параметры камеры и смещение
                    (
                        (cam_transform.translation, cam_transform.rotation),
                        Vec3::new(0.0, -0.2, 2.0)
                    )
                } else {
                    continue;
                }
            };

            // 2. Обрабатываем робота после завершения работы с камерой
            if let Ok(mut robot_transform) = transforms.get_mut(robot_entity) {
                let (cam_translation, cam_rotation) = new_cam_transform;
                robot_transform.translation = cam_translation + 
                    cam_rotation * camera_offset;
                robot_transform.rotation = Quat::from_rotation_y(
                    cam_rotation.to_euler(EulerRot::YXZ).0 - std::f32::consts::FRAC_PI_2
                );
            }
        }
    }
}