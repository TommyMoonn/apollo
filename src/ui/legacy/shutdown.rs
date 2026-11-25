//! Native close interception. The core owns cancellation and settlement; egui owns the decision.
use egui::{Align, Context, Layout, Ui, ViewportCommand};

use super::ApolloApp;
use crate::ui::{layout, theme, widgets};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExitPresentation {
    Open,
    Confirm,
    Closing,
    FailureAcknowledged,
    Authorized,
}

impl ApolloApp {
    pub(super) fn request_close(&mut self) {
        if self.exit != ExitPresentation::Open {
            return;
        }
        if self.core.queue.scheduler().pending_count() > 0 {
            self.exit = ExitPresentation::Confirm;
        } else {
            self.begin_close();
        }
    }

    pub(super) fn resolve_exit_confirmation(&mut self, confirmed: bool) {
        if self.exit != ExitPresentation::Confirm {
            return;
        }
        if confirmed {
            self.begin_close();
        } else {
            self.exit = ExitPresentation::Open;
        }
        self.egui_context.request_repaint();
    }

    fn begin_close(&mut self) {
        if self.core.begin_shutdown() {
            self.exit = ExitPresentation::Closing;
            self.confirm_cancel_queue = false;
            self.queue_frame_preview = None;
            self.egui_context.request_repaint();
        }
    }

    /// Run before rendering or advancing a queue, including when the window is occluded.
    pub(super) fn handle_close_request(&mut self, ctx: &Context) {
        if self.exit == ExitPresentation::Authorized {
            return;
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.request_close();
        }
        if self.core.is_closing() {
            let progress = &mut self.progress_wake;
            let settled = self.core.poll_shutdown(|| progress.begin());
            // Native dialogs own the window handle until they return. Do not apply their
            // results or initiate a new inspection while closing.
            self.file_dialog.poll();
            if settled
                && !self.file_dialog.is_pending()
                && (self.core.shutdown_failure().is_none()
                    || self.exit == ExitPresentation::FailureAcknowledged)
            {
                self.authorize_exit(ctx);
            }
        } else if self.exit == ExitPresentation::Confirm {
            // Existing work may settle or advance while the user decides. No new UI
            // actions are dispatched until they dismiss or accept the confirmation.
            let progress = &mut self.progress_wake;
            self.core.poll_media_queue(|| progress.begin());
        }
    }

    pub(super) fn acknowledge_shutdown_failure(&mut self) {
        if self.exit == ExitPresentation::Closing && self.core.shutdown_failure().is_some() {
            self.exit = ExitPresentation::FailureAcknowledged;
            self.egui_context.request_repaint();
        }
    }

    fn authorize_exit(&mut self, ctx: &Context) {
        if self.exit != ExitPresentation::Authorized {
            self.exit = ExitPresentation::Authorized;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    pub(super) fn show_shutdown(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(layout::root_frame(ui.style()))
            .show(ui, |ui| {
                if self.exit == ExitPresentation::Confirm {
                    // Keep accepting existing worker results while the confirmation is open.
                    self.poll_background_work();
                    let modal =
                        egui::Modal::new(egui::Id::new("confirm-exit")).show(ui.ctx(), |ui| {
                            ui.set_max_width(320.0);
                            ui.heading("Exit Apollo?");
                            ui.label("Cancel unfinished jobs and exit? Completed files are kept.");
                            ui.add_space(theme::SPACING.related);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let confirm = widgets::destructive_button(ui, "Exit").clicked();
                                let dismiss =
                                    widgets::secondary_button(ui, "Keep working").clicked();
                                (confirm, dismiss)
                            })
                            .inner
                        });
                    if modal.inner.0 {
                        self.resolve_exit_confirmation(true);
                    } else if modal.inner.1 || modal.should_close() {
                        self.resolve_exit_confirmation(false);
                    }
                    return;
                }
                layout::centered_content(ui, |ui| {
                    if let Some(error) = self.core.shutdown_failure() {
                        ui.heading("A job failed while closing");
                        ui.label(
                            "Check the diagnostic before exiting. Existing output files are kept.",
                        );
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| {
                                ui.label(error.to_string());
                            });
                        if !self.core.queue.scheduler().is_running()
                            && !self.file_dialog.is_pending()
                            && self.exit != ExitPresentation::FailureAcknowledged
                            && widgets::secondary_button(ui, "Exit").clicked()
                        {
                            // The logic pass is responsible for checking every owner before exit.
                            self.acknowledge_shutdown_failure();
                        }
                    } else {
                        ui.heading("Closing Apollo…");
                        ui.label("Waiting for background work to settle.");
                        ui.spinner();
                    }
                });
            });
    }
}
