use super::super::{HanaApp, ToastKind, VersionAction};
use super::*;
use eframe::egui::{self, RichText};

impl HanaApp {
    pub(super) fn ui_versions(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.versions_tab_downloaded, false, t.tab_all);
            ui.selectable_value(&mut self.versions_tab_downloaded, true, t.tab_downloaded);
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(t.search_versions)
                    .desired_width(220.0),
            );
            if ui
                .add_enabled(
                    !self.task_active,
                    egui::Button::new(RichText::new(t.reload).size(13.0)),
                )
                .clicked()
            {
                self.start_refresh_versions();
            }
            if self.task_active {
                ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
            }
        });
        if !self.cfg.show_all_versions {
            ui.label(
                RichText::new(t.releases_only_hint)
                    .color(TEXT_WEAK)
                    .size(11.0),
            );
        }

        if self.versions_tab_downloaded {
            self.ui_versions_downloaded(ui);
            return;
        }

        if self.versions.is_empty() && self.installed.is_empty() {
            ui.add_space(30.0);
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(if self.task_active {
                        t.loading_versions
                    } else {
                        t.empty_versions
                    })
                    .color(TEXT_WEAK),
                );
            });
            return;
        }

        let filter = self.search.to_lowercase();
        ui.add_space(4.0);

        let list = self.display_version_ids();
        let rows: Vec<usize> = (0..list.len())
            .filter(|&i| {
                let id = &list[i];
                filter.is_empty() || id.to_lowercase().contains(&filter)
            })
            .collect();

        const ROW_H: f32 = 50.0;
        let mut action: Option<VersionAction> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(false)
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                for i in range {
                    let idx = rows[i];
                    let id = &list[idx];
                    let installed = self.installed.contains(id.as_str());
                    let kind = self
                        .manifest_version(id)
                        .map(|v| v.kind.as_str())
                        .unwrap_or("installed");
                    let is_latest_active = self
                        .cfg
                        .active_installation()
                        .map(|i| i.is_latest)
                        .unwrap_or(true);
                    let is_selected = if is_latest_active {
                        self.active_resolved_version().as_deref() == Some(id.as_str())
                    } else {
                        self.cfg
                            .active_installation()
                            .and_then(|inst| inst.version_id.as_deref())
                            == Some(id.as_str())
                    };

                    egui::Frame::default()
                        .fill(if is_selected { GLASS_SOLID } else { GLASS })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if is_selected {
                                ACCENT_SOFT_STROKE
                            } else {
                                BORDER
                            },
                        ))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(id).size(14.0).strong());
                                    let kind_color = match kind {
                                        "release" => OK_GREEN,
                                        "snapshot" => WARN_YELLOW,
                                        _ => Color32::from_gray(170),
                                    };
                                    ui.label(RichText::new(kind).size(10.0).color(kind_color));
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if installed {
                                            if is_selected {
                                                ui.label(
                                                    RichText::new(t.installed)
                                                        .color(OK_GREEN)
                                                        .size(11.0),
                                                );
                                            } else if ui
                                                .add_enabled(
                                                    !self.task_active && !is_latest_active,
                                                    egui::Button::new(t.use_version),
                                                )
                                                .clicked()
                                            {
                                                action =
                                                    Some(VersionAction::Select(id.clone()));
                                            }
                                            if ui
                                                .add_enabled(
                                                    !self.task_active,
                                                    egui::Button::new(t.check_repair),
                                                )
                                                .clicked()
                                            {
                                                action =
                                                    Some(VersionAction::Repair(id.clone()));
                                            }
                                        } else if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.pick_version),
                                            )
                                            .clicked()
                                        {
                                            action =
                                                Some(VersionAction::Install(id.clone()));
                                        }
                                    },
                                );
                            });
                        });
                }
            });

        if let Some(action) = action {
            match action {
                VersionAction::Select(id) => {
                    if let Some(inst) = self.cfg.active_installation_mut() {
                        inst.version_id = Some(id);
                    }
                    let _ = crate::config::save_config(&self.cfg);
                }
                VersionAction::Install(id) => {
                    if let Some(inst) = self.cfg.active_installation_mut() {
                        if inst.is_editable() {
                            inst.version_id = Some(id.clone());
                        }
                    }
                    let _ = crate::config::save_config(&self.cfg);
                    let root = self.installation_root();
                    self.start(move |tx, drx| {
                        crate::tasks::install_version(tx, drx, id, root)
                    });
                }
                VersionAction::Repair(id) => {
                    let root = self.installation_root();
                    self.start(move |tx, drx| {
                        crate::tasks::repair_version(tx, drx, id, root)
                    });
                }
                VersionAction::DeleteData(id) => {
                    self.version_delete_confirm = Some(id);
                }
            }
        }

        self.ui_version_delete_dialog(ui);
    }

    pub(super) fn ui_versions_downloaded(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        ui.add_space(4.0);
        if self.installed.is_empty() {
            ui.add_space(30.0);
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(t.empty_downloaded).color(TEXT_WEAK));
            });
            return;
        }
        let mut installed_ids: Vec<String> = self.installed.iter().cloned().collect();
        installed_ids.sort_by(|a, b| b.cmp(a));
        let mut action: Option<VersionAction> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for id in installed_ids {
                    let is_selected =
                        self.active_resolved_version().as_deref() == Some(id.as_str());
                    let is_latest = self.latest_stable().as_deref() == Some(id.as_str());
                    egui::Frame::default()
                        .fill(if is_selected { ACCENT_SOFT } else { GLASS })
                        .stroke(egui::Stroke::new(1.0, BORDER))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&id).size(14.0).strong());
                                if is_latest {
                                    ui.label(
                                        RichText::new(t.latest_badge)
                                            .size(9.0)
                                            .color(ACCENT)
                                            .strong(),
                                    );
                                }
                                if is_selected {
                                    ui.label(
                                        RichText::new(t.installed)
                                            .color(OK_GREEN)
                                            .size(11.0),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.check_repair),
                                            )
                                            .clicked()
                                        {
                                            action =
                                                Some(VersionAction::Repair(id.clone()));
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.task_active,
                                                egui::Button::new(t.delete_data),
                                            )
                                            .clicked()
                                        {
                                            action =
                                                Some(VersionAction::DeleteData(id.clone()));
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        if let Some(action) = action {
            match action {
                VersionAction::Repair(id) => {
                    let root = self.installation_root();
                    self.start(move |tx, drx| {
                        crate::tasks::repair_version(tx, drx, id, root)
                    });
                }
                VersionAction::DeleteData(id) => {
                    self.version_delete_confirm = Some(id);
                }
                _ => {}
            }
        }

        self.ui_version_delete_dialog(ui);
    }

    pub(super) fn ui_version_delete_dialog(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let Some(id) = self.version_delete_confirm.take() else {
            return;
        };
        let mut open = true;
        let mut do_delete = false;
        let ctx = ui.ctx().clone();

        let modal = egui::Modal::new(egui::Id::new("version_delete_modal")).show(&ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(t.version_delete_title).size(15.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(t.version_delete_body.replace("{}", &id)).size(12.5),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(t.delete).clicked() {
                    do_delete = true;
                }
                if ui.button(t.cancel).clicked() {
                    open = false;
                }
            });
        });
        if modal.should_close() {
            open = false;
        }

        if do_delete {
            let dir = crate::install::version_dir(&self.installation_root(), &id);
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            self.refresh_installed();
            self.toast(ToastKind::Ok, t.version_deleted);
        } else if open {
            self.version_delete_confirm = Some(id);
        }
    }
}
