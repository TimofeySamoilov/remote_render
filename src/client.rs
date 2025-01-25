use eframe::egui::{self, TextureOptions};
use rdev::{listen, EventType};
use remote_render::greeter_client::GreeterClient;
use remote_render::key_board_control_client::KeyBoardControlClient;
use remote_render::ControlRequest;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
pub mod remote_render {
    tonic::include_proto!("remote_render");
}
struct ScreenApp {
    screen_length: usize,
    screen_height: usize,
    pixels: Vec<u8>,
    receiver: mpsc::Receiver<Vec<u8>>,
    stop_sender: mpsc::Sender<bool>,
}

#[tokio::main]
async fn main() -> eframe::Result<()> {
    eframe::run_native(
        "App",
        eframe::NativeOptions::default(),
        Box::new(|_cc| {
            let app = ScreenApp::default();
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

// default() has an async fn, which is connecting to server
impl Default for ScreenApp {
    fn default() -> Self {
        let shared_pixels: Vec<u8> = vec![0; 900 * 900 * 4];
        let (tx, rx): (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) = mpsc::channel(10000);
        let shared_tx = tx.clone(); // clone sender for use in the async block
        let (stop_tx, stop_rx): (mpsc::Sender<bool>, mpsc::Receiver<bool>) = mpsc::channel(1);
        let stop_rx = Arc::new(Mutex::new(stop_rx));
        let stop_rx_clone = Arc::clone(&stop_rx);
        tokio::spawn(async move {
            match GreeterClient::connect("http://[::1]:50051").await {
                Ok(mut client) => {
                    let _communication =
                        streaming_data(&mut client, shared_tx, stop_rx_clone).await;

                    Ok::<(), crate::egui::Key>(())
                }
                Err(e) => {
                    eprintln!("Error connecting to server: {}", e);
                    Err(crate::egui::Key::A) // return a default error
                }
            }
        });

        tokio::spawn(async move {
            match KeyBoardControlClient::connect("http://[::1]:50051").await {
                Ok(mut client) => {
                    let pressed_key = Arc::new(Mutex::new(String::from("")));
                    let pressed_key_clone = Arc::clone(&pressed_key);
                    let _keyboard_check = tokio::spawn(keyboard(pressed_key, stop_rx));
                    loop {
                        let key_clone = pressed_key_clone.lock().unwrap().to_string();

                        match client
                            .say_keyboard(ControlRequest {
                                message: key_clone.clone(),
                            })
                            .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                println!("Error sending key: {}", e)
                            }
                        }

                        tokio::time::sleep(Duration::from_millis(8)).await;
                    }
                }
                Err(e) => {
                    println!("Error connecting to server: {}", e);
                }
            }
        });
        // making ScreenApp
        ScreenApp {
            screen_length: 900,
            screen_height: 900,
            pixels: shared_pixels,
            receiver: rx,
            stop_sender: stop_tx,
        }
    }
}

impl eframe::App for ScreenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // copy data from receiver channel
            while let Ok(data) = self.receiver.try_recv() {
                self.pixels.copy_from_slice(&data);
            }
            // making texture from pixels
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.screen_length, self.screen_height],
                &self.pixels,
            );
            let texture = ctx.load_texture("my_texture", image, TextureOptions::default());
            ui.image(&texture);
            ctx.request_repaint();
        });
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.stop_sender.try_send(true);
        std::process::exit(0);
    }
}

// client's part
async fn streaming_data(
    client: &mut GreeterClient<Channel>,
    sender: mpsc::Sender<Vec<u8>>,
    _stop_rx: Arc<Mutex<mpsc::Receiver<bool>>>,
) {
    let mut stream = client
        .say_hello(ControlRequest {
            message: "client connected".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    let frames = Arc::new(Mutex::new(0u64));
    let frames_clone = frames.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("Client FRAMES: {}", frames_clone.lock().unwrap());
            *frames_clone.lock().unwrap() = 0;
        }
    });
    while let Some(item) = stream.next().await {
        let _ = sender.send(item.unwrap().message.to_vec()).await;
        let mut frames_guard = frames.lock().unwrap();
        *frames_guard += 1;
    }
}

// function monitoring keyboard buttons
async fn keyboard(shared_string: Arc<Mutex<String>>, stop_rx: Arc<Mutex<mpsc::Receiver<bool>>>) {
    listen(move |event| {
        while let Ok(_data) = stop_rx.lock().unwrap().try_recv() {
            println!("Keyboard is closed");
            return;
        }
        match event.event_type {
            EventType::KeyPress(key) => {
                let mut s = shared_string.lock().unwrap();
                *s = format!("{:?}", key);
            }
            _ => {
                let mut s = shared_string.lock().unwrap();
                *s = "No one is pressed".to_string();
            }
        }
    })
    .unwrap();
}
