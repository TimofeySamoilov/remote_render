use bevy::{
    prelude::*,
};
use std::{
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};
//use std::collections::HashMap;

use bevy_tokio_tasks::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::key_board_control_server::{KeyBoardControl, KeyBoardControlServer};
use remote_render::{ControlRequest, FrameDispatch};
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
    pub receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
    pub senders: Vec<mpsc::Sender<ChannelMessage>>,
    pub clients: Arc<Mutex<Vec<u32>>>,
    pub frame_numbers: Arc<Mutex<Vec<u32>>>, // Теперь для каждого клиента свой frame_number
    pub lz4_compression: bool,
    pub pressed_key: Arc<Mutex<u8>>,
}

impl Default for Communication {
    fn default() -> Self {
        // Создаем 4 канала для 4 клиентов
        let mut senders = Vec::new();
        let mut receivers = Vec::new();
        
        for _ in 0..4 {
            let (sender, receiver) = mpsc::channel(10000);
            senders.push(sender);
            receivers.push(Arc::new(Mutex::new(receiver)));
        }

        Communication {
            receivers: Arc::new(Mutex::new(receivers)),
            senders,
            clients: Arc::new(Mutex::new(vec![])),
            frame_numbers: Arc::new(Mutex::new(vec![0; 4])), // Инициализируем для 4 клиентов
            lz4_compression: false,
            pressed_key: Arc::new(Mutex::new(0u8)),
        }
    }
}
#[derive(Debug)]
struct RenderService {
    frame_receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
    clients: Arc<Mutex<Vec<u32>>>,
}
impl RenderService {
    pub fn new(
        receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
        clients: Arc<Mutex<Vec<u32>>>,
    ) -> Self {
        RenderService {
            frame_receivers: receivers,
            clients: clients,
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
        let request = request.into_inner();
        let message = request.message;
        //let id = request.id;
        //println!("{:?}", message);
        /*println!("ID ID ID ID    {:?}", id);
        let mut key = self.pressed_key.lock().unwrap();
        *key = compare_key(message);*/
        Ok(Response::new(ControlRequest {
            message: "OK".to_string(),
            id: 0,
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
        let (tx, rx) = mpsc::channel(128);
        let client_id = request.into_inner().id;
        
        // Регистрация клиента и получение camera_id
        let camera_id = {
            let mut clients = self.clients.lock().unwrap();
            clients.push(client_id);
            clients.len().checked_sub(1).unwrap_or(0)
        };
        // Получаем нужный receiver
        let receiver = {
            let receivers = self.frame_receivers.lock().unwrap();
            receivers.get(camera_id)
                .ok_or_else(|| Status::internal("No receiver for this camera"))?
                .clone()
        };
        
        tokio::spawn(async move {
            loop {
                let message = {
                    let mut guard = receiver.lock().unwrap();
                    guard.try_recv()
                };
                
                let (screen, frame) = match message {
                    Ok(ChannelMessage::PixelsAndFrame(pixels, frame_id)) => (pixels, frame_id),
                    _ => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        continue;
                    }
                };
                
                if let Err(e) = tx.try_send(Ok(FrameDispatch { message: screen, frame })) {
                    trace!("Frame skip: {:?}", e);
                }
            }
        });
        
        Ok(Response::new(Box::pin(ReceiverStream::new(rx)) as _))
    }
}
// function, which starts sever
async fn start_server(
    receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
    clients: Arc<Mutex<Vec<u32>>>,
    pressed_key: Arc<Mutex<u8>>
) -> Result<(), Box<dyn Error>> {
    let greeter_addr = "[::1]:50051".parse()?;
    let greeter = RenderService::new(receivers, clients);
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
    let receivers_clone = communication.receivers.clone();
    let clients = communication.clients.clone();
    let pressed_key = communication.pressed_key.clone();
    runtime.spawn_background_task(|_ctx| async move {
        match start_server(receivers_clone, clients, pressed_key).await {
            Ok(_) => {
                info!("Server started...")
            }
            Err(e) => {
                error!("Error with starting server: {:?}", e);
            }
        }
    });
}