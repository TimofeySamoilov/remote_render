//#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] hide console window on Windows in release

use eframe::egui::{self, TextureOptions};  
  
fn main() -> eframe::Result<()>  {  
    eframe::run_native(  
        "App",  
        eframe::NativeOptions::default(),  
        Box::new(|cc| {
            Ok(Box::new(ScreenApp::new(cc)) as Box<dyn eframe::App>)
        }),
    )  
} 
  
struct ScreenApp {screen_length: usize, screen_height: usize}
impl ScreenApp {  
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {  
        ScreenApp {screen_length: 1920, screen_height: 1080}  
    }  
}  
//impl eframe::App for ScreenApp {}
impl eframe::App for ScreenApp {  
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {  
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut pixels: Vec<u8> = vec![0; self.screen_length * self.screen_height*4];
            //let mut pixel_array: Vec<egui::Rect> = Vec::new();
            
            for y in 0..self.screen_height {
                for x in 0..self.screen_length {
                    let index = (y * self.screen_length + x) * 4; // Учитываем 4 компонента
                    pixels[index] = 0;     // Красный
                    pixels[index + 1] = 255; // Зеленый
                    pixels[index + 2] = 0; // Синий
                    pixels[index + 3] = 128; // Альфа
                }
            }
            

            // Создание текстуры из пикселей
            let image = egui::ColorImage::from_rgba_unmultiplied([self.screen_length, self.screen_height], &pixels);
            let texture = ctx.load_texture("my_texture", image, TextureOptions::default());
            ui.image(&texture);
        });  
    }  
}
