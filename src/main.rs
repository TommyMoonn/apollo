mod app;
use app::ApolloApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([620.0, 410.0])
            .with_min_inner_size([400.0, 260.0]),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "apollo",
        native_options,
        Box::new(|creation_context| Ok(Box::new(ApolloApp::new(creation_context)))),
    )
}
