use super::super::{HanaApp, ToastKind};
use super::*;
use eframe::egui;

impl HanaApp {
    pub(super) fn ui_console(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.horizontal(|ui| {
            if ui.button(t.console_clear).clicked() {
                self.console.clear();
            }
            if ui.button(t.console_save).clicked() {
                let logs_dir = self.root.join("logs");
                let _ = std::fs::create_dir_all(&logs_dir);
                let path = logs_dir.join(format!("console-{}.log", now_stamp()));
                if std::fs::write(&path, &self.console).is_ok() {
                    self.toast(
                        ToastKind::Ok,
                        t.log_saved.replace("{}", &path.to_string_lossy()),
                    );
                } else {
                    self.toast(
                        ToastKind::Error,
                        t.save_failed.replace("{}", &path.to_string_lossy()),
                    );
                }
            }
            if ui.button(t.copy).clicked() {
                ui.ctx().copy_text(self.console.clone());
                self.toast(ToastKind::Info, t.log_copied);
            }
            if ui.button(t.open_logs_folder).clicked() {
                let logs_dir = self.root.join("logs");
                let _ = std::fs::create_dir_all(&logs_dir);
                if !open_in_explorer(&logs_dir) {
                    self.toast(
                        ToastKind::Error,
                        t.open_failed
                            .replace("{}", &logs_dir.to_string_lossy()),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.console_scroll, t.auto_scroll);
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.console_scroll)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.console)
                        .code_editor()
                        .desired_rows(30)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
    }
}
