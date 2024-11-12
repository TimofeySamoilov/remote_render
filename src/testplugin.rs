use bevy::prelude::*;
//use chrono::prelude::*;
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::{HelloReply, HelloRequest};
use std::{error::Error, pin::Pin, time::Duration};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use tonic::{transport::Server, Request, Response, Status};
//server
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
            // Создаем бесконечный цикл
            while let Ok(_) = tx
                .send(Result::<_, Status>::Ok(HelloReply {
                    message: chrono::prelude::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                }))
                .await
            {
                // Ждем 16 миллисекунд перед отправкой следующего сообщения
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::SayHelloStream
        ))
    }
}

//plugin
pub struct MyPlugin {
    runtime: Runtime,
}

impl MyPlugin {
    pub fn send(&self) {
        self.runtime.spawn(async {
            while (1 == 1) {
                println!("hiii");
            }
        });
    }
}
pub async fn start_server() -> Result<(), Box<dyn Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter {};
    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;
    println!("server");
    Ok(())
}
impl Plugin for MyPlugin {
    fn build(&self, app: &mut App) {
        self.runtime.spawn(async move {
            match start_server().await {
                Err(e) => {println!("error{:?}", e)},
                _ => {println!("Ok!")},
            }
        });
    }
}

fn main() {
    let async_plugin = MyPlugin {
        runtime: Runtime::new().expect("msg"),
    };


    std::env::set_var("WGPU_BACKEND", "vulkan");

    App::new().add_plugins(async_plugin).run();
    std::thread::sleep(Duration::from_secs(10));

}

