//! Retained egui presentation and behavior contracts for later migration phases.
//! The GPUI executable does not construct this adapter.

mod audio_extract;
mod audio_mix;
mod remux;
mod upscale;

mod batch;
mod dialogs;
mod frame_capture;
mod queue;
mod render;
mod shutdown;
mod source;
mod state;
mod workflows;

use egui::{Context, Ui};

use crate::app::dialogs::InputDialogOrigin;
use crate::app::{Application, discovery::MediaToolsState};
use crate::file_dialog::FileDialogOwner;
use crate::progress_wake::MediaProgressWakeOwner;
#[cfg(test)]
use crate::settings::AppSettings;

use state::{EditorPresentation, LegacyEditorState};

pub struct ApolloApp {
    core: Application,
    state: LegacyEditorState,
    queue_editor_presentation: Option<EditorPresentation>,
    confirm_cancel_queue: bool,
    exit: shutdown::ExitPresentation,
    queue_frame_preview: Option<queue::QueueFramePreview>,
    batch_editor_presentation: Option<EditorPresentation>,
    batch_append_error_details_open: bool,
    progress_wake: MediaProgressWakeOwner,
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
    #[cfg(test)]
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
            confirm_cancel_queue: false,
            exit: shutdown::ExitPresentation::Open,
            queue_frame_preview: None,
            batch_editor_presentation: None,
            batch_append_error_details_open: false,
            progress_wake: MediaProgressWakeOwner::new(completion_wake(&egui_context)),
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
            self.core.refresh_batch_capabilities();
        }
        if !matches!(
            self.core.navigation(),
            crate::app::NavigationState::Queue { .. }
        ) {
            self.queue_frame_preview = None;
        }
        let progress = &mut self.progress_wake;
        self.core.poll_media_queue(|| progress.begin());
        if !matches!(self.core.discovery.state(), MediaToolsState::Ready(_)) {
            return;
        }
        self.poll_file_dialog();
        self.poll_media_inspection();
        self.core.poll_batch_inspection();
        self.poll_frame_capture_preview();
        self.poll_frame_capture_index();
        self.poll_copy_frame();
    }

    // Defensive fallback only; normal close waits for settlement before Drop.
    fn shutdown(&mut self) {
        self.cancel_copy_frame_work();
        self.cancel_frame_capture_support();
        self.core.retire_batch();
        self.core.queue.cancel();
    }
}

impl Drop for ApolloApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl eframe::App for ApolloApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.exit != shutdown::ExitPresentation::Open {
            self.show_shutdown(ui);
        } else {
            self.show_root(ui);
        }
    }
}

#[cfg(test)]
mod tests;
