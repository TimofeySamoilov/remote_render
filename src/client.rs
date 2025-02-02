use eframe::egui::{self, TextureOptions, TextureHandle, ColorImage};
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

use tracing_subscriber::{filter::EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use tracing::{span, Level, trace};
use tracing_subscriber::fmt;

struct ScreenApp {
    screen_length: usize,
    screen_height: usize,
    pixels: Vec<u8>,
    receiver: mpsc::Receiver<ChannelMessage>,
    stop_sender: mpsc::Sender<bool>,
    texture: Option<TextureHandle>,
    ctx: egui::Context,

}
enum ChannelMessage {
    Pixels(Vec<u8>),
    PixelsAndFrame(Vec<u8>, u32)
}

#[tokio::main]
async fn main() -> eframe::Result<()> {
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
    eframe::run_native(
        "App",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            let app = ScreenApp::new(cc.egui_ctx.clone());
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

// default() has an async fn, which is connecting to server
impl ScreenApp {
    fn new(ctx: egui::Context) -> Self {
        let shared_pixels: Vec<u8> = vec![0; 900 * 900 * 4];
        let (tx, rx): (mpsc::Sender<ChannelMessage>, mpsc::Receiver<ChannelMessage>) = mpsc::channel(10000);
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
        let texture = {
            let image = ColorImage::from_rgba_unmultiplied(
                 [900, 900],
                &shared_pixels,
            );
            ctx.load_texture("my_texture", image, TextureOptions::default())
        };
        // making ScreenApp
        ScreenApp {
            screen_length: 900,
            screen_height: 900,
            pixels: shared_pixels,
            receiver: rx,
            stop_sender: stop_tx,
            texture: Some(texture),
            ctx: ctx
        }
    }
}

impl eframe::App for ScreenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // copy data from receiver channel
            let mut frame: u32 = 0;
            if let Ok(data) = self.receiver.try_recv() {
                match data {
                    ChannelMessage::PixelsAndFrame(pixels, id) => {
                        self.pixels.copy_from_slice(&pixels);
                        frame = id;
                        if let Some(_texture) = &mut self.texture {
                            let image = ColorImage::from_rgba_unmultiplied(
                                [self.screen_length, self.screen_height],
                                &self.pixels,
                            );
                            self.texture = Some(self.ctx.load_texture("my_texture", image, TextureOptions::default()));
                        }
                    }
                    _ => {

                    }
                }
            }
            if frame > 0 {
                let span = span!(Level::TRACE, "frame", n = frame);
                let _enter = span.enter();
                trace!("frame printed");
            }
            // making texture from pixels
            // making texture from pixels
            if let Some(texture) = &self.texture {
                ui.image(texture);
            }
            
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
    sender: mpsc::Sender<ChannelMessage>,
    _stop_rx: Arc<Mutex<mpsc::Receiver<bool>>>,
) {
    let mut stream = client
        .say_hello(ControlRequest {
            message: "client connected".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    while let Some(item) = stream.next().await {
        let data = item.unwrap();
        let span = span!(Level::TRACE, "frame", n = data.frame);
        let _enter = span.enter();
        trace!("received a frame by server");
        match sender.try_send(ChannelMessage::PixelsAndFrame(data.message.to_vec(), data.frame)) {
            Ok(_) => {trace!("frame sent to egui")},
            Err(e) => {trace!("error with sending to egui! {:?}", e)}
        };
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
