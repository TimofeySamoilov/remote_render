use bevy::{
    prelude::*,
};
use std::{
    sync::{

        Arc,
    },
    time::Duration,
};

use bevy_tokio_tasks::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::key_board_control_server::{KeyBoardControl, KeyBoardControlServer};
use remote_render::{ControlRequest, FrameDispatch};
use std::sync::Mutex;
use std::{error::Error, pin::Pin};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};

use tracing::{span, Level};

pub mod remote_render {
    tonic::include_proto!("remote_render");
}
#[derive(Resource)]
pub struct Communication {
    pub receiver: Arc<Mutex<mpsc::Receiver<ChannelMessage>>>,
    pub sender: mpsc::Sender<ChannelMessage>,
    pub client_connected: Arc<Mutex<bool>>,
    pub frame_number: Arc<Mutex<u32>>,
    pub lz4_compression: bool,
    pub pressed_key: Arc<Mutex<u8>>
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
pub enum ChannelMessage {
    Pixels(Vec<u8>),
    PixelsAndFrame(Vec<u8>, u32)
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
pub fn compare_key(s: String) -> u8 {
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
pub fn server_system(runtime: ResMut<TokioTasksRuntime>, communication: ResMut<Communication>) {
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