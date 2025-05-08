use bevy::prelude::*;
use std::{
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};

use bevy_tokio_tasks::*;
use remote_render::connection_server::{Connection, ConnectionServer};
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
pub struct CamerasMovement {
    pub button_sender: Arc<Mutex<mpsc::Sender<(u32, String)>>>,
    pub button_receiver: mpsc::Receiver<(u32, String)>
}

impl Default for CamerasMovement {
    fn default() -> Self {
        let (button_sender, button_receiver): (mpsc::Sender<(u32, String)>, mpsc::Receiver<(u32, String)>) = mpsc::channel(1000);
        CamerasMovement {
            button_sender: Arc::new(Mutex::new(button_sender)),
            button_receiver: button_receiver
        }
    }
}


#[derive(Resource)]
pub struct Communication {
    pub receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
    pub senders: Vec<mpsc::Sender<ChannelMessage>>,
    pub clients: Arc<Mutex<Vec<u32>>>,
    pub frame_numbers: Arc<Mutex<Vec<u32>>>, // Теперь для каждого клиента свой frame_number
    pub lz4_compression: bool,
}

impl Default for Communication {
    fn default() -> Self {
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
            frame_numbers: Arc::new(Mutex::new(vec![0; 4])),
            lz4_compression: false,
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
    button_sender: Arc<Mutex<mpsc::Sender<(u32, String)>>>,
}
impl KeyBoardButtons {
    pub fn new(button_sender: Arc<Mutex<mpsc::Sender<(u32, String)>>>) -> Self {
        KeyBoardButtons {
            button_sender: button_sender
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
        let sender = self.button_sender.lock().unwrap();
        let _ = sender.try_send((request.id, request.message));
        Ok(Response::new(ControlRequest {
            message: "OK".to_string(),
            id: 0,
        }))
    }
}

#[tonic::async_trait]
impl Connection for RenderService {
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

// system, which uses start_server()
pub fn server_system(runtime: ResMut<TokioTasksRuntime>, communication: ResMut<Communication>, cameras_movement: ResMut<CamerasMovement>) {
    let receivers_clone = communication.receivers.clone();
    let clients = communication.clients.clone();
    let button_sender = cameras_movement.button_sender.clone();
    runtime.spawn_background_task(|_ctx| async move {
        match start_server(receivers_clone, clients, button_sender).await {
            Ok(_) => {
                info!("Server started...")
            }
            Err(e) => {
                error!("Error with starting server: {:?}", e);
            }
        }
    });
}

// function, which starts sever
async fn start_server(
    receivers: Arc<Mutex<Vec<Arc<Mutex<mpsc::Receiver<ChannelMessage>>>>>>,
    clients: Arc<Mutex<Vec<u32>>>,
    button_sender: Arc<Mutex<mpsc::Sender<(u32, String)>>>
) -> Result<(), Box<dyn Error>> {
    let connection_addr = "[::1]:50051".parse()?;
    let connection = RenderService::new(receivers, clients);
    let keyboard = KeyBoardButtons::new(button_sender);
    
    tokio::spawn(async move {
        Server::builder()
            .add_service(ConnectionServer::new(connection))
            .add_service(KeyBoardControlServer::new(keyboard))
            .serve(connection_addr)
            .await
            .expect("Server error");
    });
    Ok(())
}

