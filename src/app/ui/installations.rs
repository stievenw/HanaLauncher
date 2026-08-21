use super::super::{HanaApp, DeleteDraft, InstallationDraft, ToastKind};
use super::*;
use crate::config::Installation;
use eframe::egui::{self, RichText};

impl HanaApp {
    pub(super) fn ui_installations(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let busy = self.busy();

        ui.horizontal(|ui| {
            ui.label(RichText::new(t.installations_title).size(9.0).color(TEXT_WEAK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.task_active,
                        egui::Button::new(RichText::new(t.new_installation).size(13.0)),
                    )
                    .clicked()
                {
                    let mut d = InstallationDraft::new();
                    d.version_id = self
                        .cfg
                        .active_installation()
                        .and_then(|i| i.version_id.clone());
                    self.installation_dialog = Some(d);
                }
            });
        });
        ui.add_space(4.0);

        if self.cfg.installations.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new(t.no_installations).color(TEXT_WEAK));
                ui.label(
                    RichText::new(t.no_installations_hint)
                        .color(TEXT_WEAK)
                        .size(11.0),
                );
            });
        } else {
            let mut select_idx: Option<usize> = None;
            let mut edit_idx: Option<usize> = None;
            let mut delete_idx: Option<usize> = None;
            let mut open_folder_idx: Option<usize> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, inst) in self.cfg.installations.iter().enumerate() {
                        let is_active =
                            self.cfg.active_installation.as_deref() == Some(inst.name.as_str());
                        let ver =self.installation_resolved_version(inst)
                            .unwrap_or_else(|| "-".to_string());
                        let inst_ok =self.installation_resolved_version(inst)
                            .as_ref()
                            .map(|id| self.installed.contains(id))
                            .unwrap_or(false);

                        egui::Frame::default()
                            .fill(if is_active { ACCENT_SOFT } else { GLASS })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if is_active {
                                    ACCENT_SOFT_STROKE
                                } else {
                                    BORDER
                                },
                            ))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(self.installation_display_name(inst))
                                                    .size(14.0)
                                                    .strong(),
                                            );
                                            if inst.is_latest {
                                                ui.label(
                                                    RichText::new(t.latest_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                            if is_active {
                                                ui.label(
                                                    RichText::new(t.active_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                        });
                                        let status = if inst_ok {
                                            t.installed
                                        } else {
                                            t.not_installed
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{ver}  -  {status}  -  {} MB",
                                                inst.memory_mb
                                            ))
                                            .color(TEXT_WEAK)
                                            .size(11.0),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.open_folder),
                                                )
                                                .clicked()
                                            {
                                                open_folder_idx = Some(i);
                                            }
                                            if inst.is_latest {
                                                // The built-in "latest" instance is
                                                // managed automatically; it offers no
                                                // delete / delete-data button.
                                            } else if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.delete),
                                                )
                                                .clicked()
                                            {
                                                delete_idx = Some(i);
                                            }
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.edit),
                                                )
                                                .clicked()
                                            {
                                                edit_idx = Some(i);
                                            }
                                            if !is_active
                                                && ui
                                                    .add_enabled(
                                                        !busy,
                                                        egui::Button::new(t.select),
                                                    )
                                                    .clicked()
                                            {
                                                select_idx = Some(i);
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(5.0);
                    }
                });

            if let Some(idx) = select_idx {
                self.cfg.active_installation = Some(self.cfg.installations[idx].name.clone());
                let _ = crate::config::save_config(&self.cfg);
                self.toast(ToastKind::Ok, t.installation_updated);
            }
            if let Some(idx) = edit_idx {
                let inst = self.cfg.installations[idx].clone();
                self.installation_dialog = Some(InstallationDraft::from_installation(idx, &inst));
            }
            if let Some(idx) = delete_idx {
                let inst = &self.cfg.installations[idx];
                self.delete_dialog = Some(DeleteDraft {
                    idx,
                    name: inst.name.clone(),
                    also_folder: false,
                    deletable: false,
                });
            }
            if open_folder_idx.is_some() {
                let dir = self.cfg.launcher_dir();
                let _ = std::fs::create_dir_all(&dir);
                if !open_in_explorer(&dir) {
                    self.toast(
                        ToastKind::Error,
                        t.open_failed.replace("{}", &dir.to_string_lossy()),
                    );
                }
            }
        }

        self.ui_installation_dialog(ui);
        self.ui_delete_dialog(ui);
    }

    pub(super) fn ui_installation_dialog(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let Some(draft) = self.installation_dialog.take() else {
            return;
        };
        let mut draft = draft;
        let mut open = true;
        let mut save = false;
        let ctx = ui.ctx().clone();

        let title = if draft.editing.is_some() {
            t.edit
        } else {
            t.new_installation
        };
        let modal = egui::Modal::new(egui::Id::new("inst_edit_modal")).show(&ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(title).size(15.0).strong());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(t.installation_name);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.name).desired_width(240.0),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.mc_version);
                if draft.is_latest {
                    ui.label(
                        RichText::new(t.latest_no_edit)
                            .size(13.0)
                            .color(TEXT_WEAK),
                    );
                } else {
                    let sel = draft.version_id.clone().unwrap_or_default();
                    let mut new_sel = sel.clone();
                    egui::ComboBox::from_id_salt("installation_dialog_version")
                        .selected_text(RichText::new(&sel).size(14.0).color(TEXT))
                        .width(240.0)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.version_search)
                                        .hint_text(t.search_versions)
                                        .desired_width(220.0),
                                );
                            });
                            let filter = draft.version_search.to_lowercase();
                            let list: Vec<String> = self
                                .display_version_ids()
                                .into_iter()
                                .filter(|id| {
                                    filter.is_empty()
                                        || id.to_lowercase().contains(&filter)
                                })
                                .collect();
                            const ROW_H: f32 = 24.0;
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .show_rows(ui, ROW_H, list.len(), |ui, range| {
                                    for i in range {
                                        let id = &list[i];
                                        let kind = self
                                            .manifest_version(id)
                                            .map(|v| {
                                                if v.kind == "release" {
                                                    "Release".to_string()
                                                } else {
                                                    v.kind.clone()
                                                }
                                            })
                                            .unwrap_or_else(|| "installed".to_string());
                                        let label = RichText::new(format!(
                                            "{id}  [{kind}]"
                                        ))
                                        .size(13.0)
                                        .color(TEXT);
                                        ui.selectable_value(
                                            &mut new_sel,
                                            id.clone(),
                                            label,
                                        );
                                    }
                                });
                        });
                    if new_sel != sel {
                        draft.version_id = Some(new_sel);
                        egui::Popup::close_id(
                            ui.ctx(),
                            egui::Id::new("installation_dialog_version").with("popup"),
                        );
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.memory_label);
                ui.add(
                    egui::Slider::new(&mut draft.memory_mb, 1024..=16384)
                        .text("MB")
                        .suffix(" MB"),
                );
            });
            ui.add_space(4.0);
            ui.checkbox(&mut draft.download_java, t.auto_download_java);
            ui.horizontal(|ui| {
                ui.label(t.java_path);
                let path = draft.java_path.get_or_insert_with(|| String::new());
                ui.add(
                    egui::TextEdit::singleline(path).desired_width(240.0),
                );
                if ui.button(t.detect).clicked() {
                    if let Some(p) =
                        crate::java::detect_java(draft.java_path.as_deref(), &self.root)
                    {
                        draft.java_path = Some(p.to_string_lossy().into_owned());
                    }
                }
                if ui.button(t.clear).clicked() {
                    draft.java_path = None;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.resolution);
                ui.add(egui::DragValue::new(&mut draft.width).range(320..=7680));
                ui.label("x");
                ui.add(egui::DragValue::new(&mut draft.height).range(240..=4320));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.extra_jvm_args_label);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.extra_jvm_args)
                        .desired_width(240.0)
                        .hint_text(t.extra_jvm_args_hint),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t.authlib_url_label);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.authlib_url)
                        .desired_width(200.0)
                        .hint_text("ely.by"),
                );
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    t.launcher_dir_current
                        .replace("{}", &self.cfg.launcher_dir().to_string_lossy()),
                )
                .color(TEXT_WEAK)
                .size(10.5),
            );
            ui.label(
                RichText::new(t.launcher_dir_hint)
                    .color(TEXT_WEAK)
                    .size(10.5),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t.save).clicked() {
                    save = true;
                }
                if ui.button(t.cancel).clicked() {
                    open = false;
                }
            });
        });
        let esc = modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            });
        if esc {
            open = false;
        }

        if save {
            let name = draft.name.trim().to_string();
            let is_latest_edit = draft
                .editing
                .map(|idx| self.cfg.installations[idx].is_latest)
                .unwrap_or(false);
            if name == crate::config::LATEST_INSTALLATION_KEY && !is_latest_edit {
                self.toast(ToastKind::Error, t.latest_key_reserved);
                self.installation_dialog = Some(draft);
                return;
            }
            let name_taken = name.is_empty()
                || self
                    .cfg
                    .installations
                    .iter()
                    .enumerate()
                    .any(|(i, x)| x.name == name && Some(i) != draft.editing);
            if name_taken {
                self.toast(ToastKind::Error, t.name_taken);
                self.installation_dialog = Some(draft);
                return;
            }
            if let Some(idx) = draft.editing {
                let old_name = self.cfg.installations[idx].name.clone();
                let was_active =
                    self.cfg.active_installation.as_deref() == Some(old_name.as_str());
                let inst = &mut self.cfg.installations[idx];
                inst.name = name.clone();
                if !inst.is_latest {
                    inst.version_id = draft.version_id.clone();
                }
                inst.memory_mb = draft.memory_mb;
                inst.java_path = draft.java_path.clone();
                inst.download_java = draft.download_java;
                inst.width = draft.width;
                inst.height = draft.height;
                inst.extra_jvm_args = draft.extra_jvm_args.clone();
                inst.authlib_url = draft.authlib_url.clone();
                if was_active {
                    self.cfg.active_installation = Some(name);
                }
                self.toast(ToastKind::Ok, t.installation_updated);
            } else {
                let mut inst = Installation::new(name);
                inst.version_id = draft.version_id;
                inst.memory_mb = draft.memory_mb;
                inst.java_path = draft.java_path;
                inst.download_java = draft.download_java;
                inst.width = draft.width;
                inst.height = draft.height;
                inst.extra_jvm_args = draft.extra_jvm_args;
                inst.authlib_url = draft.authlib_url;
                let idx = self.cfg.installations.len();
                self.cfg.installations.push(inst);
                self.cfg.active_installation =
                    Some(self.cfg.installations[idx].name.clone());
                self.toast(ToastKind::Ok, t.installation_created);
            }
            let _ = crate::config::save_config(&self.cfg);
            self.refresh_active_root();
            self.installation_dialog = None;
            return;
        }
        if !open {
            self.installation_dialog = None;
        } else {
            self.installation_dialog = Some(draft);
        }
    }

    pub(super) fn ui_delete_dialog(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let Some(mut draft) = self.delete_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut do_delete = false;
        let ctx = ui.ctx().clone();

        let modal =
            egui::Modal::new(egui::Id::new("inst_delete_modal")).show(&ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new(t.delete_confirm_title).size(15.0).strong());
                ui.add_space(6.0);
                ui.label(
                    RichText::new(t.delete_confirm_body.replace("{}", &draft.name))
                        .size(12.5),
                );
                if draft.deletable {
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut draft.also_folder,
                        t.delete_confirm_also_folder,
                    );
                } else {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(t.delete_folder_locked)
                            .color(TEXT_WEAK)
                            .size(11.0),
                    );
                }
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
            let was_active =
                self.cfg.active_installation.as_deref() == Some(draft.name.as_str());
            if draft.also_folder && draft.deletable {
                let dir = self.cfg.launcher_dir();
                if dir.exists() {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                self.toast(ToastKind::Ok, t.folder_deleted);
            }
            self.cfg.installations.remove(draft.idx);
            if was_active {
                self.cfg.normalize_active_installation();
            }
            let _ = crate::config::save_config(&self.cfg);
            self.toast(ToastKind::Ok, t.installation_deleted);
        } else if !open {
            self.delete_dialog = None;
        } else {
            self.delete_dialog = Some(draft);
        }
    }
}
