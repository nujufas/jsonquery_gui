mod app;
mod tree_view;
mod worker;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 420.0])
            .with_title("jsonquery"),
        ..Default::default()
    };

    eframe::run_native(
        "jsonquery",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
