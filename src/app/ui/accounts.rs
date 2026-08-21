use super::super::{HanaApp, ToastKind};
use super::*;
use crate::config::{
    ACCOUNT_TYPE_ELY_OAUTH, ACCOUNT_TYPE_ELY_PASSWORD, ACCOUNT_TYPE_OFFLINE,
};
use crate::worker::LaunchDecision;
use eframe::egui::{self, RichText, Vec2};
use std::sync::mpsc::channel as new_channel;

impl HanaApp {
    pub(super) fn ui_accounts(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();
        let busy = self.busy();

        // ---- Add account (always visible at the top, compact) ----
        ui.label(RichText::new(t.add_account).size(9.0).color(TEXT_WEAK));
        ui.add_space(4.0);

        let online_ok = self.has_valid_online();
        let any_busy = self.task_active;

        card(ui, |ui| {
            ui.horizontal(|ui| {
                // LEFT: Ely.by OAuth login
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t.login_with_ely).strong().size(12.5));
                    ui.label(
                        RichText::new(t.login_ely_account)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                    ui.add_space(5.0);
                    if ui
                        .add_enabled(
                            !any_busy,
                            egui::Button::new(
                                RichText::new(t.ely_oauth)
                                    .size(13.0)
                                    .strong()
                                    .color(PRIMARY_TEXT),
                            )
                            .fill(PRIMARY_BTN)
                            .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        self.start(|tx, drx| crate::tasks::login_oauth(tx, drx));
                    }
                    ui.add_space(4.0);
                    ui.hyperlink_to(
                        RichText::new(t.register_ely).size(10.5).color(ACCENT),
                        crate::util::ELY_REGISTER_URL,
                    );
                });
                ui.separator();
                // RIGHT: Add offline account. Requires a valid Ely.by account.
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t.offline_account).strong().size(12.5));
                    ui.label(
                        RichText::new(t.offline_desc)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.offline_name)
                                .hint_text(t.offline_name)
                                .desired_width(150.0),
                        );
                        if ui
                            .add_enabled(
                                online_ok && !any_busy,
                                egui::Button::new(t.add_offline),
                            )
                            .clicked()
                        {
                            let name = self.offline_name.trim().to_string();
                            let valid = (3..=16).contains(&name.len())
                                && name
                                    .chars()
                                    .all(|c| c.is_alphanumeric() || c == '_');
                            if !valid {
                                self.toast(ToastKind::Error, t.offline_name_invalid);
                            } else {
                                self.offline_name.clear();
                                self.add_or_replace_account(
                                    crate::auth::offline_account(&name),
                                );
                            }
                        }
                    });
                    if !online_ok {
                        ui.label(
                            RichText::new(t.offline_locked)
                                .color(ERR_RED)
                                .size(10.5),
                        );
                    }
                });
            });
        });

        ui.add_space(8.0);

        card(ui, |ui| {
            egui::CollapsingHeader::new(
                RichText::new(t.password_title).strong().size(13.0),
            )
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_username)
                        .hint_text(t.email_or_username)
                        .desired_width(300.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_password)
                        .hint_text(t.password)
                        .password(!self.pw_show)
                        .desired_width(300.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.pw_show, t.show_password)
                        .changed()
                    {
                        let _ = self.pw_show;
                    }
                    if ui
                        .add_enabled(!self.task_active, egui::Button::new(t.sign_in))
                        .clicked()
                    {
                        let username = self.pw_username.trim().to_string();
                        let password = self.pw_password.clone();
                        if username.is_empty() || password.is_empty() {
                            self.toast(ToastKind::Error, t.fill_credentials);
                        } else {
                            self.start(move |tx, drx| {
                                crate::tasks::login_password(
                                    tx, drx, username, password, None,
                                );
                            });
                        }
                    }
                });
            });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // ---- Saved accounts ----
        if self.cfg.accounts.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new(t.no_saved_accounts).color(TEXT_WEAK));
            });
            ui.add_space(10.0);
        } else {
            ui.label(RichText::new(t.saved_accounts).size(9.0).color(TEXT_WEAK));
            ui.add_space(4.0);

            let n = self.cfg.accounts.len();
            let mut remove_idx: Option<usize> = None;
            let mut activate_idx: Option<usize> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for i in 0..n {
                        let is_active = self.cfg.active_account_index == Some(i);
                        let acc = self.cfg.accounts[i].clone();
                        let offline_locked = !online_ok
                            && acc.account_type == ACCOUNT_TYPE_OFFLINE;

                        egui::Frame::default()
                            .fill(if is_active && !offline_locked {
                                ACCENT_SOFT
                            } else {
                                GLASS
                            })
                            .stroke(egui::Stroke::new(
                                1.0,
                                if is_active && !offline_locked {
                                    ACCENT_SOFT_STROKE
                                } else {
                                    BORDER
                                },
                            ))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if let Some(tex) = self.avatars.get(&acc.uuid) {
                                        ui.image((tex.id(), Vec2::new(34.0, 34.0)));
                                    } else {
                                        ui.allocate_ui(Vec2::new(34.0, 34.0), |ui| {
                                            ui.centered_and_justified(|ui| {
                                                ui.label(
                                                    RichText::new(
                                                        acc.username
                                                            .chars()
                                                            .next()
                                                            .unwrap_or('?')
                                                            .to_string(),
                                                    )
                                                    .size(14.0)
                                                    .strong(),
                                                );
                                            });
                                        });
                                    }
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(&acc.username)
                                                    .size(13.5)
                                                    .strong(),
                                            );
                                            if is_active {
                                                ui.label(
                                                    RichText::new(t.active_badge)
                                                        .size(9.0)
                                                        .color(ACCENT)
                                                        .strong(),
                                                );
                                            }
                                            let ty = if acc.account_type
                                                == ACCOUNT_TYPE_ELY_OAUTH
                                            {
                                                t.ely_oauth
                                            } else if acc.account_type
                                                == ACCOUNT_TYPE_ELY_PASSWORD
                                            {
                                                t.ely_password
                                            } else {
                                                t.offline_account
                                            };
                                            ui.label(
                                                RichText::new(ty)
                                                    .color(TEXT_WEAK)
                                                    .size(10.5),
                                            );
                                        });
                                        if offline_locked {
                                            ui.label(
                                                RichText::new(t.offline_locked)
                                                    .color(ERR_RED)
                                                    .size(10.5),
                                            );
                                        }
                                        if let Some(exp) = acc.expires_at {
                                            let now = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_secs() as i64;
                                            let left = exp - now;
                                            let detail = t
                                                .token_expires_on
                                                .replace(
                                                    "{}",
                                                    &format_expiry_date(exp),
                                                )
                                                .replacen(
                                                    "{}",
                                                    &format_time_left(left.max(0), t),
                                                    1,
                                                );
                                            let color = if left <= 0 {
                                                ERR_RED
                                            } else if left < 86400 {
                                                Color32::from_rgb(235, 180, 60)
                                            } else {
                                                TEXT_WEAK
                                            };
                                            ui.label(
                                                RichText::new(detail)
                                                    .color(color)
                                                    .size(11.0),
                                            );
                                        }
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(t.remove),
                                                )
                                                .clicked()
                                            {
                                                remove_idx = Some(i);
                                            }
                                            if !is_active
                                                && !offline_locked
                                                && ui
                                                    .add_enabled(
                                                        !busy,
                                                        egui::Button::new(
                                                            t.activate,
                                                        ),
                                                    )
                                                    .clicked()
                                            {
                                                activate_idx = Some(i);
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(5.0);
                    }
                });

            if let Some(idx) = activate_idx {
                self.cfg.active_account_index = Some(idx);
                let _ = crate::config::save_config(&self.cfg);
                let uuid = self.cfg.accounts[idx].uuid.clone();
                let (_, drx) = new_channel::<LaunchDecision>();
                crate::tasks::fetch_avatar(self.tx.clone(), drx, uuid);
                self.toast(
                    ToastKind::Ok,
                    t.account_active
                        .replace("{}", &self.cfg.accounts[idx].username),
                );
            }
            if let Some(idx) = remove_idx {
                self.remove_account(idx);
            }
        }
    }
}
