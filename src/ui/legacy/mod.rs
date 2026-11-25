//! Legacy egui composition. The library core owns route and non-rendering state.

mod audio_extract;
mod audio_mix;
mod remux;
mod upscale;

mod dialogs;
mod frame_capture;
mod preferences;
mod queue;
mod render;
mod source;
mod state;
mod workflows;

use egui::{Context, Ui};

use crate::app::dialogs::InputDialogOrigin;
use crate::app::{Application, discovery::MediaToolsState};
use crate::file_dialog::FileDialogOwner;
use crate::native::WindowOwner;
use crate::progress_repaint::MediaProgressRepaintOwner;
use crate::settings::AppSettings;
use crate::ui::{fonts, theme};

use state::{
    BatchEditState, BatchInspectionSession, BatchSetupState, EditorPresentation, LegacyEditorState,
};

pub struct ApolloApp {
    core: Application,
    state: LegacyEditorState,
    queue_editor_presentation: Option<EditorPresentation>,
    batch_setup: Option<BatchSetupState>,
    batch_edit: Option<BatchEditState>,
    batch_inspection: Option<BatchInspectionSession>,
    progress_repaint: MediaProgressRepaintOwner,
    file_dialog: FileDialogOwner,
    clipboard_owner: Option<isize>,
    input_dialog_origin: Option<InputDialogOrigin>,
    egui_context: Context,
    tools_error_details_open: bool,
}

fn completion_wake(context: &Context) -> impl Fn() + Clone + Send + 'static {
    let context = context.clone();
    move || context.request_repaint()
}

impl ApolloApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        fonts::install(&creation_context.egui_ctx);
        theme::install(&creation_context.egui_ctx);

        let egui_context = creation_context.egui_ctx.clone();
        let native_window_owner = WindowOwner::from_window(creation_context);
        #[cfg(target_os = "windows")]
        let clipboard_owner = native_window_owner
            .as_ref()
            .ok()
            .copied()
            .map(WindowOwner::native_handle);
        #[cfg(not(target_os = "windows"))]
        let clipboard_owner = None;
        let file_dialog = FileDialogOwner::new(completion_wake(&egui_context), native_window_owner);
        let settings = preferences::load(creation_context.storage);
        let mut app =
            Self::with_adapters(egui_context.clone(), file_dialog, clipboard_owner, settings);
        app.retry_media_tools();
        app
    }

    fn with_adapters(
        egui_context: Context,
        file_dialog: FileDialogOwner,
        clipboard_owner: Option<isize>,
        settings: AppSettings,
    ) -> Self {
        Self {
            core: Application::with_settings(settings),
            state: LegacyEditorState::default(),
            queue_editor_presentation: None,
            batch_setup: None,
            batch_edit: None,
            batch_inspection: None,
            progress_repaint: MediaProgressRepaintOwner::new(completion_wake(&egui_context)),
            file_dialog,
            clipboard_owner,
            input_dialog_origin: None,
            egui_context,
            tools_error_details_open: false,
        }
    }

    fn poll_background_work(&mut self) {
        if self.core.discovery.poll() {
            self.tools_error_details_open = false;
            // Batch defaults prepared while capabilities were pending must be reconsidered on settlement.
            if let Some(batch) = self.batch_setup.as_mut()
                && batch.operation == crate::app::Operation::VideoUpscale
            {
                workflows::refresh_batch_items(batch, self.core.discovery.upscale_capabilities());
            }
        }
        let progress = &mut self.progress_repaint;
        self.core.poll_media_queue(|| progress.begin());
        if !matches!(self.core.discovery.state(), MediaToolsState::Ready(_)) {
            return;
        }
        self.poll_file_dialog();
        self.poll_media_inspection();
        self.poll_batch_inspection();
        self.poll_frame_capture_preview();
        self.poll_frame_capture_index();
        self.poll_copy_frame();
        self.poll_frame_capture_thumbnail();
    }

    fn shutdown(&mut self) {
        self.cancel_copy_frame_work();
        self.cancel_frame_capture_support();
        self.core.frame_work.cancel_thumbnail();
        self.core.queue.cancel();
    }
}

impl Drop for ApolloApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl eframe::App for ApolloApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show_root(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        preferences::save(&self.core.settings, storage);
    }
}

#[cfg(test)]
mod tests;
