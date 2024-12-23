use bevy::prelude::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::{HelloReply, HelloRequest};
use std::{error::Error, pin::Pin, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};
use bevy_tokio_tasks::*;

pub mod remote_render {
    tonic::include_proto!("remote_render");
}

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    type SayHelloStream = Pin<Box<dyn Stream<Item = Result<HelloReply, Status>> + Send>>;
    
    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<Response<Self::SayHelloStream>, Status> {
        println!("{:?}", request.into_inner().message);

        let (tx, rx) = mpsc::channel(128);
        
        tokio::spawn(async move {
            let mut a: u8 = 0;
            let mut screen = vec![0; 500 * 500 * 4];
            while let Ok(_) = tx.send(Result::<_, Status>::Ok(HelloReply {
                message: screen.clone(),
            })).await {
                tokio::time::sleep(Duration::from_millis(128)).await;
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
        Ok(Response::new(Box::pin(output_stream) as Self::SayHelloStream))
    }
}


async fn start_server() -> Result<(), Box<dyn Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter {};
    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;
    Ok(())
}


fn server_system(runtime: ResMut<TokioTasksRuntime>) {
    runtime.spawn_background_task(|_ctx| async move {
        match start_server().await {
            Ok(_) => {println!("OKKK");},
            Err(e) => {eprintln!("{:?}", e);}
        }
    });
}

fn main() {
    std::env::set_var("WGPU_BACKEND", "vulkan");
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default())
        .add_systems(Startup, server_system)
        .run();
}

fn set_color(a: u8) -> (u8, u8, u8) {
    match a {
        0 => (255, 0, 0),
        1 => (0, 255, 0),
        _ => (0, 0, 255),
    }
}
