#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
use apollo::ui::layout;
use app::ApolloApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(layout::REFERENCE_CONTENT_SIZE)
            .with_min_inner_size(layout::MINIMUM_CONTENT_SIZE)
            .with_icon(apollo::ui::icon::application_icon()),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Apollo",
        native_options,
        Box::new(|creation_context| {
            let app = ApolloApp::new(creation_context);
            Ok(Box::new(app))
        }),
    )
}
