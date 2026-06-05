use egui::Ui;

const AUTH_URL: &str = "https://twitchaddon.page.link/1Sk5";

#[derive(Default)]
pub struct AuthView {
    pub token_input: String,
}

impl AuthView {
    pub fn ui(&mut self, ui: &mut Ui, on_token_saved: &mut Option<String>) {
        ui.heading("Authentication Required");
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
            let config = crate::config::Config {
                access_token: Some(token.clone()),
                last_quality: None,
            };
            if let Err(e) = config.save() {
                eprintln!("Failed to save config: {}", e);
            } else {
                *on_token_saved = Some(token);
            }
        }
    }
}
