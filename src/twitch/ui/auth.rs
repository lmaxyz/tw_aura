use egui::Ui;

const AUTH_URL: &str = "https://twitchaddon.page.link/1Sk5";

#[derive(Default)]
pub struct AuthView {
    pub token_input: String,
    /// Пояснение, почему требуется авторизация (например, истёкший токен).
    pub notice: Option<String>,
}

impl AuthView {
    pub fn ui(&mut self, ui: &mut Ui, on_token_saved: &mut Option<String>) {
        ui.heading("Authentication Required");
        if let Some(notice) = &self.notice {
            ui.colored_label(egui::Color32::YELLOW, notice);
            ui.add_space(8.0);
        }
        ui.label("Please authenticate with Twitch to use this app.");
        ui.add_space(8.0);

        if ui
            .link(
                egui::RichText::new("Open Twitch Auth page")
                    .heading()
                    .color(ui.ctx().theme().default_visuals().hyperlink_color),
            )
            .clicked()
        {
            #[cfg(feature = "aurora")]
            aurora_services::open_uri::open_uri(AUTH_URL, |_| {
                // Do something with the response
            });
            #[cfg(not(feature = "aurora"))]
            ui.ctx().open_url(egui::OpenUrl::new_tab(AUTH_URL));
        }

        ui.add_space(16.0);
        ui.label("Enter your access token:");
        ui.text_edit_singleline(&mut self.token_input);

        ui.add_space(8.0);
        if ui.button("Save Token").clicked() && !self.token_input.trim().is_empty() {
            let token = self.token_input.trim().to_owned();
            let mut config = crate::config::Config::load().unwrap_or_default();
            config.access_token = Some(token.clone());
            if let Err(e) = config.save() {
                log::error!("Failed to save config: {e}");
            } else {
                self.notice = None;
                self.token_input.clear();
                *on_token_saved = Some(token);
            }
        }
    }
}
