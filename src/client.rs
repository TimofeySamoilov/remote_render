use remote_render::greeter_client::GreeterClient;
use remote_render::HelloRequest;
use tokio_stream::{StreamExt};
use tonic::transport::Channel;
use std::sync::{Arc, Mutex};
use rdev::{listen, EventType};
use eframe::egui::{self, TextureOptions};
use tokio::sync::mpsc;

pub mod remote_render { tonic::include_proto!("remote_render"); }

struct ScreenApp {screen_length: usize, screen_height: usize, pixels: Vec<u8>, receiver: mpsc::Receiver<Vec<u8>>}

#[tokio::main]
async fn main() -> eframe::Result<()> {
    eframe::run_native(
        "App",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            let app = ScreenApp::default(); 
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}


// default() has an async fn, which is connecting to server 
impl Default for ScreenApp {
    fn default() -> Self {
        let shared_pixels: Vec<u8> = vec![0; 500 * 500 *4];
        let (tx, rx): (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) = mpsc::channel(1);
        let shared_tx = tx.clone(); // Clone sender for use in the async block
        tokio::spawn(async move {
            match GreeterClient::connect("http://[::1]:50051").await {
               Ok(mut client) => {
                   let pressed_key = Arc::new(Mutex::new(String::from("")));
                   let pressed_key_clone = Arc::clone(&pressed_key);
                   let _keyboard_check = tokio::spawn(keyboard(pressed_key));
                   let _communication = streaming_data(&mut client, 10000000, pressed_key_clone, shared_tx).await;
                 
                   Ok::<(), crate::egui::Key>(())
               },
               Err(e) => {
                   eprintln!("Error connecting to server: {}", e);
                  Err(crate::egui::Key::A) // return a default error
               }
           }
       });
        ScreenApp {screen_length: 500, screen_height: 500, pixels: shared_pixels, receiver: rx}
    }
}  

impl eframe::App for ScreenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("SuperWindow");
            while let Ok(data) = self.receiver.try_recv() {
                self.pixels.copy_from_slice(&data);
            }
            // Создание текстуры из пикселей
            let image = egui::ColorImage::from_rgba_unmultiplied([self.screen_length, self.screen_height], &self.pixels);
            let texture = ctx.load_texture("my_texture", image, TextureOptions::default());
            ui.image(&texture);
            ctx.request_repaint();
        });
    }  
}

//Client's part
async fn streaming_data(client: &mut GreeterClient<Channel>, num: usize, pressed_key: Arc<Mutex<String>>, sender: mpsc::Sender<Vec<u8>>) {
    let stream = client.say_hello( HelloRequest { message: "client connected".to_string()}).await.unwrap().into_inner();
    let mut stream = stream.take(num);
    while let Some(item) = stream.next().await {
        let _ = sender.send(item.unwrap().message.to_vec()).await;
        client.say_hello( HelloRequest { message: format!("{:?}", pressed_key.lock()).to_string()}).await.unwrap().into_inner();
    }
}

//Function are monitoring keyboard buttons
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