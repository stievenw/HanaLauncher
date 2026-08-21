use super::super::{HanaApp, Page};
use super::*;
use crate::config::{ACCOUNT_TYPE_ELY_OAUTH, ACCOUNT_TYPE_ELY_PASSWORD};
use eframe::egui::{self, RichText, Vec2};

impl HanaApp {
    pub(super) fn ui_home(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();

        // ----- active installation -----
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(t.nav_installations.to_uppercase())
                            .size(9.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(2.0);
                    let inst_name = self
                        .cfg
                        .active_installation()
                        .map(|i| self.installation_display_name(i))
                        .unwrap_or_else(|| "-".to_string());
                    ui.label(RichText::new(&inst_name).size(18.0).strong());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t.manage).clicked() {
                        self.page = Page::Installations;
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(t.choose_installation).size(12.0).color(TEXT_WEAK));
                let active = self
                    .cfg
                    .active_installation
                    .clone()
                    .unwrap_or_default();
                let mut new_active = active.clone();
                egui::ComboBox::from_id_salt("home_instance")
                    .selected_text(
                        RichText::new(
                            self.cfg
                                .active_installation()
                                .map(|i| self.installation_display_name(i))
                                .unwrap_or_else(|| "-".to_string()),
                        )
                        .size(14.0)
                        .strong()
                        .color(TEXT),
                    )
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for inst in &self.cfg.installations {
                            ui.selectable_value(
                                &mut new_active,
                                inst.name.clone(),
                                self.installation_display_name(inst),
                            );
                        }
                    });
                if new_active != active {
                    self.cfg.active_installation = Some(new_active);
                    let _ = crate::config::save_config(&self.cfg);
                }
            });
        });

        ui.add_space(10.0);

        // ----- resolved version -----
        let sel = self
            .active_resolved_version()
            .unwrap_or_else(|| "-".to_string());
        let is_latest = self
            .cfg
            .active_installation()
            .map(|i| i.is_latest)
            .unwrap_or(false);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(t.mc_version).size(9.0).color(TEXT_WEAK));
                    ui.add_space(2.0);
                    ui.label(RichText::new(&sel).size(15.0).strong().color(TEXT));
                    ui.add_space(4.0);
                    if is_latest {
                        ui.label(RichText::new(t.latest_hint).size(11.0).color(TEXT_WEAK));
                    } else if sel == "-" {
                        ui.label(
                            RichText::new(t.create_installation_hint)
                                .size(11.0)
                                .color(TEXT_WEAK),
                        );
                    }
                });
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t.status).size(9.0).color(TEXT_WEAK));
                    ui.add_space(2.0);
                    let status = if self.active_version_installed() {
                        RichText::new(t.installed).color(OK_GREEN)
                    } else if sel == "-" {
                        RichText::new(t.not_installed).color(TEXT_WEAK)
                    } else {
                        RichText::new(t.not_installed).color(WARN_YELLOW)
                    };
                    ui.label(status.size(13.0).strong());
                });
            });
        });

        ui.add_space(10.0);

        // ----- play / install (single button) -----
        let installed = self.active_version_installed();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let play_text = if self.game_running() {
                    t.stop
                } else if !installed {
                    t.install
                } else {
                    t.play
                };
                let has_version = self.active_resolved_version().is_some();
                let play_enabled = self.game_running()
                    || (has_version
                        && (!installed || self.cfg.active_account().is_some()));
                let play = egui::Button::new(
                    RichText::new(play_text)
                        .size(18.0)
                        .strong()
                        .color(PRIMARY_TEXT),
                )
                .fill(if self.game_running() {
                    ERR_RED
                } else {
                    PRIMARY_BTN
                })
                .corner_radius(10.0)
                .min_size(Vec2::new(160.0, 46.0));
                if ui
                    .add_enabled(play_enabled && !self.task_active, play)
                    .clicked()
                {
                    self.play_clicked();
                }

                if self.busy() {
                    ui.label(
                        RichText::new(t.waiting_task)
                            .color(TEXT_WEAK)
                            .size(12.0),
                    );
                } else if self.cfg.active_installation().is_none() {
                    ui.label(
                        RichText::new(t.no_installation_selected)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                } else if !has_version {
                    ui.label(
                        RichText::new(t.need_version_hint)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                } else if !installed && self.cfg.active_account().is_none() {
                    ui.label(
                        RichText::new(t.need_login_hint)
                            .color(WARN_YELLOW)
                            .size(12.0),
                    );
                }
            });
        });

        ui.add_space(10.0);

        // ----- active account -----
        ui.label(RichText::new(t.active_account).size(9.0).color(TEXT_WEAK));
        ui.add_space(4.0);
        card(ui, |ui| match self.cfg.active_account() {
            Some(acc) => {
                ui.horizontal(|ui| {
                    if let Some(tex) = self.avatars.get(&acc.uuid) {
                        ui.image((tex.id(), Vec2::new(38.0, 38.0)));
                    } else {
                        ui.allocate_ui(Vec2::new(38.0, 38.0), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(
                                        acc.username.chars().next().unwrap_or('?').to_string(),
                                    )
                                    .size(16.0)
                                    .strong(),
                                );
                            });
                        });
                    }
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&acc.username).size(15.0).strong());
                        let ty = if acc.account_type == ACCOUNT_TYPE_ELY_OAUTH {
                            t.ely_oauth
                        } else if acc.account_type == ACCOUNT_TYPE_ELY_PASSWORD {
                            t.ely_password
                        } else {
                            t.offline_account
                        };
                        ui.label(RichText::new(ty).size(11.0).color(TEXT_WEAK));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t.manage).clicked() {
                            self.page = Page::Accounts;
                        }
                    });
                });
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.no_active_account).color(TEXT_WEAK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t.sign_in).clicked() {
                            self.page = Page::Accounts;
                        }
                    });
                });
            }
        });
    }
}
