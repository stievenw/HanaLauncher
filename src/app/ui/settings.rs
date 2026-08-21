use super::super::{HanaApp, ToastKind};
use super::*;
use eframe::egui::{self, RichText};

impl HanaApp {
    pub(super) fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let t = self.cfg.t();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card(ui, |ui| {
                    ui.label(
                        RichText::new(t.vanilla_branding_label)
                            .size(9.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut self.cfg.vanilla_branding, t.vanilla_branding_label)
                        .changed()
                    {
                        let _ = crate::config::save_config(&self.cfg);
                        self.toast(ToastKind::Info, t.saved);
                    }
                    ui.label(
                        RichText::new(t.vanilla_branding_hint)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(
                        RichText::new(t.versions_label)
                            .size(9.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut self.cfg.show_all_versions, t.show_all_versions)
                        .changed()
                    {
                        let _ = crate::config::save_config(&self.cfg);
                    }
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(
                        RichText::new(t.launcher_dir_label)
                            .size(9.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            t.launcher_dir_current
                                .replace("{}", &self.cfg.launcher_dir().to_string_lossy()),
                        )
                        .color(TEXT_WEAK)
                        .size(10.5),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(t.open_folder).clicked() {
                            let dir = self.cfg.launcher_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            if !open_in_explorer(&dir) {
                                self.toast(
                                    ToastKind::Error,
                                    t.open_failed
                                        .replace("{}", &dir.to_string_lossy()),
                                );
                            }
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t.launcher_dir_hint)
                            .color(TEXT_WEAK)
                            .size(10.5),
                    );
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(
                        RichText::new(t.language_label)
                            .size(9.0)
                            .color(TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t.language_choice);
                        let mut lang = self.cfg.language.clone();
                        egui::ComboBox::from_id_salt("lang_choice")
                            .selected_text(RichText::new(
                                if lang == crate::config::LANG_EN {
                                    t.english
                                } else {
                                    t.indonesian
                                },
                            ))
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut lang,
                                    crate::config::LANG_ID.to_string(),
                                    t.indonesian,
                                );
                                ui.selectable_value(
                                    &mut lang,
                                    crate::config::LANG_EN.to_string(),
                                    t.english,
                                );
                            });
                        if lang != self.cfg.language {
                            self.cfg.language = lang;
                            let _ = crate::config::save_config(&self.cfg);
                            crate::lang::set_current(self.cfg.t());
                            self.toast(ToastKind::Info, self.cfg.t().saved);
                        }
                    });
                });

                ui.add_space(8.0);

                card(ui, |ui| {
                    ui.label(
                        RichText::new(t.font_label).size(9.0).color(TEXT_WEAK),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t.font_choice);
                        let mut font_mode = self.cfg.font_mode.clone();
                        egui::ComboBox::from_id_salt("font_choice")
                            .selected_text(RichText::new(
                                if font_mode == crate::config::FontMode::Monogram {
                                    t.font_monogram
                                } else {
                                    t.font_system
                                },
                            ))
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut font_mode,
                                    crate::config::FontMode::Monogram,
                                    t.font_monogram,
                                );
                                ui.selectable_value(
                                    &mut font_mode,
                                    crate::config::FontMode::System,
                                    t.font_system,
                                );
                            });
                        if font_mode != self.cfg.font_mode {
                            self.cfg.font_mode = font_mode;
                            let _ = crate::config::save_config(&self.cfg);
                            apply_font_mode(ui.ctx(), &self.cfg.font_mode);
                            self.toast(ToastKind::Info, self.cfg.t().saved);
                        }
                    });
                });

                ui.add_space(12.0);

                card(ui, |ui| {
                    ui.label(RichText::new(t.credits_title).size(9.0).color(TEXT_WEAK));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t.credits_developer).size(12.0).color(TEXT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t.credits_font_title)
                            .size(12.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(t.credits_font_desc)
                            .size(11.0)
                            .color(TEXT_WEAK),
                    );
                    ui.hyperlink_to(
                        RichText::new(t.credits_font_license)
                            .size(11.0)
                            .color(ACCENT),
                        t.credits_font_license,
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t.credits_thanks_title)
                            .size(12.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(t.credits_thanks_desc)
                            .size(11.0)
                            .color(TEXT_WEAK),
                    );
                });
            });
    }
}
