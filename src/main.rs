mod app;
use apollo::ui::layout;
use app::ApolloApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(layout::REFERENCE_CONTENT_SIZE)
            .with_min_inner_size(layout::MINIMUM_CONTENT_SIZE),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "apollo",
        native_options,
        Box::new(|creation_context| {
            let app = ApolloApp::new(creation_context)?;
            Ok(Box::new(app))
        }),
    )
}
