use tokio::sync::mpsc;
use std::{pin::Pin, error::Error, net::ToSocketAddrs, time::Duration};
use tonic::{transport::Server, Request, Response, Status};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use remote_render::greeter_server::{Greeter, GreeterServer};
use remote_render::{HelloReply, HelloRequest};
use chrono::prelude::*;

pub mod remote_render { tonic::include_proto!("remote_render");}
type StreamResult<T> = Result<Response<T>, Status>;

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    type SayHelloStream = Pin<Box<dyn Stream<Item = Result<HelloReply, Status>> + Send>>;
    
    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<Response<Self::SayHelloStream>, Status> {
        println!("{:?}", request.into_inner().message);

        let (tx, rx) = mpsc::channel(128);
        
        tokio::spawn(async move {
            // Создаем бесконечный цикл
            while let Ok(_) = tx.send(Result::<_, Status>::Ok(HelloReply {
                message: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            })).await {
                // Ждем 16 миллисекунд перед отправкой следующего сообщения
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(output_stream) as Self::SayHelloStream))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter {};

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}