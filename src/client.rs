use std::time::Duration;

use remote_render::greeter_client::GreeterClient;
use remote_render::{HelloRequest, HelloReply};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::Channel;
use std::sync::{Arc, Mutex};
use rdev::{listen, EventType};

pub mod remote_render { tonic::include_proto!("remote_render"); }

async fn streaming_data(client: &mut GreeterClient<Channel>, num: usize, string2: Arc<Mutex<String>>) {
    let stream = client.say_hello( HelloRequest { message: "client".to_string()}).await.unwrap().into_inner();
    let mut stream = stream.take(num);
    while let Some(item) = stream.next().await {
        println!("Received from server {:?}", item.unwrap().message);
        let messag = client.say_hello( HelloRequest { message: format!("{:?}", string2).to_string()}).await.unwrap().into_inner();
    }
}
async fn keyboard(shared_string: Arc<Mutex<String>>) {
    listen (move |event| {
        match event.event_type {
            EventType::KeyPress(key) => {
                //println!("Key is pressed: {:?}", key);
                let mut s = shared_string.lock().unwrap();
                *s = format!("{:?}", key);
                if key == rdev::Key::Escape {
                    println!("Process is over");
                    std::process::exit(0);
                }
            }
            _ => { let mut s = shared_string.lock().unwrap();
            *s = "No one is pressed".to_string(); }
        }
    }).unwrap();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = GreeterClient::connect("http://[::1]:50051").await.unwrap();
    let string1 = Arc::new(Mutex::new(String::from("")));
    let string2 = Arc::clone(&string1);
    let f = tokio::spawn(keyboard(string1));
    let cl = streaming_data(&mut client, 1000000, string2).await;
    tokio::time::sleep(Duration::from_millis(2)).await;
    Ok(())
}
