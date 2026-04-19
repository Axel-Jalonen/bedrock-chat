use std::collections::HashMap;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use rand::Rng;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::bedrock::{self, StreamToken};
use crate::db::Database;
use crate::message::{ChatMessage, Conversation, Role, TokenUsage, MODELS, REGIONS};

// ── Theme-aware color palette ──────────────────────────────────────────

#[derive(Clone)]
struct Palette {
    bg_base: egui::Color32,
    bg_sidebar: egui::Color32,
    bg_user_msg: egui::Color32,
    bg_assist_msg: egui::Color32,
    bg_input: egui::Color32,
    bg_topbar: egui::Color32,
    bg_modal: egui::Color32,
    accent: egui::Color32,
    accent_dim: egui::Color32,
    text_primary: egui::Color32,
    text_secondary: egui::Color32,
    text_muted: egui::Color32,
    error: egui::Color32,
    border: egui::Color32,
    hover: egui::Color32,
    selected: egui::Color32,
    role_user: egui::Color32,
    role_assistant: egui::Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg_base:       egui::Color32::from_rgb(22, 22, 26),
            bg_sidebar:    egui::Color32::from_rgb(28, 28, 33),
            bg_user_msg:   egui::Color32::from_rgb(32, 33, 42),
            bg_assist_msg: egui::Color32::from_rgb(26, 26, 30),
            bg_input:      egui::Color32::from_rgb(34, 35, 40),
            bg_topbar:     egui::Color32::from_rgb(28, 28, 33),
            bg_modal:      egui::Color32::from_rgb(32, 33, 38),
            accent:        egui::Color32::from_rgb(100, 140, 255),
            accent_dim:    egui::Color32::from_rgb(70, 100, 190),
            text_primary:  egui::Color32::from_rgb(220, 222, 228),
            text_secondary:egui::Color32::from_rgb(140, 144, 158),
            text_muted:    egui::Color32::from_rgb(90, 94, 108),
            error:         egui::Color32::from_rgb(255, 110, 110),
            border:        egui::Color32::from_rgb(50, 52, 60),
            hover:         egui::Color32::from_rgb(42, 44, 54),
            selected:      egui::Color32::from_rgb(40, 50, 75),
            role_user:     egui::Color32::from_rgb(130, 170, 255),
            role_assistant:egui::Color32::from_rgb(160, 220, 160),
        }
    }

    fn light() -> Self {
        Self {
            bg_base:       egui::Color32::from_rgb(245, 245, 248),
            bg_sidebar:    egui::Color32::from_rgb(235, 236, 240),
            bg_user_msg:   egui::Color32::from_rgb(225, 230, 245),
            bg_assist_msg: egui::Color32::from_rgb(240, 240, 244),
            bg_input:      egui::Color32::from_rgb(255, 255, 255),
            bg_topbar:     egui::Color32::from_rgb(235, 236, 240),
            bg_modal:      egui::Color32::from_rgb(255, 255, 255),
            accent:        egui::Color32::from_rgb(50, 100, 220),
            accent_dim:    egui::Color32::from_rgb(130, 160, 220),
            text_primary:  egui::Color32::from_rgb(30, 30, 36),
            text_secondary:egui::Color32::from_rgb(90, 94, 108),
            text_muted:    egui::Color32::from_rgb(150, 154, 168),
            error:         egui::Color32::from_rgb(200, 50, 50),
            border:        egui::Color32::from_rgb(210, 212, 220),
            hover:         egui::Color32::from_rgb(220, 222, 230),
            selected:      egui::Color32::from_rgb(210, 220, 245),
            role_user:     egui::Color32::from_rgb(40, 80, 180),
            role_assistant:egui::Color32::from_rgb(30, 130, 50),
        }
    }

    fn for_theme(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Dark => Self::dark(),
            egui::Theme::Light => Self::light(),
        }
    }
}

// ── Particle system ────────────────────────────────────────────────────

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    radius: f32,
    alpha: f32,
}

struct Particles {
    list: Vec<Particle>,
    time: f64,
}

impl Particles {
    fn new(count: usize) -> Self {
        let mut rng = rand::thread_rng();
        let list = (0..count)
            .map(|_| Particle {
                x: rng.gen_range(0.0..1.0),
                y: rng.gen_range(0.0..1.0),
                vx: rng.gen_range(-0.003..0.003),
                vy: rng.gen_range(-0.002..0.002),
                radius: rng.gen_range(1.5..4.0),
                alpha: rng.gen_range(0.15..0.45),
            })
            .collect();
        Self { list, time: 0.0 }
    }

    fn update_and_draw(&mut self, painter: &egui::Painter, rect: egui::Rect, base_color: egui::Color32, dt: f32) {
        self.time += dt as f64;
        for p in &mut self.list {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            // Wrap around
            if p.x < 0.0 { p.x += 1.0; }
            if p.x > 1.0 { p.x -= 1.0; }
            if p.y < 0.0 { p.y += 1.0; }
            if p.y > 1.0 { p.y -= 1.0; }

            let screen_x = rect.left() + p.x * rect.width();
            let screen_y = rect.top() + p.y * rect.height();
            let wave = ((self.time * 0.5 + p.x as f64 * 3.0).sin() * 0.5 + 0.5) as f32;
            let a = (p.alpha * wave * 255.0) as u8;
            let color = egui::Color32::from_rgba_premultiplied(
                base_color.r(),
                base_color.g(),
                base_color.b(),
                a,
            );
            painter.circle_filled(egui::pos2(screen_x, screen_y), p.radius, color);
        }
    }
}

// ── App state ──────────────────────────────────────────────────────────

#[derive(Default)]
struct CredentialForm {
    api_key: String,
    region_idx: usize,
}

enum Screen {
    Credentials(CredentialForm),
    Chat,
}

pub struct ChatApp {
    rt: tokio::runtime::Handle,
    db: Database,
    screen: Screen,

    conversations: Vec<Conversation>,
    active_id: Option<String>,
    messages: Vec<ChatMessage>,
    md_caches: HashMap<String, (u64, CommonMarkCache)>,
    streaming_md_cache: CommonMarkCache,
    input: String,
    stream_rx: Option<mpsc::UnboundedReceiver<StreamToken>>,
    is_streaming: bool,
    last_error: Option<String>,
    scroll_to_bottom: bool,
    model_idx: usize,
    region_idx: usize,
    show_system_prompt: bool,
    clipboard: Option<arboard::Clipboard>,

    /// Accumulated token usage for the active conversation
    conv_usage: TokenUsage,
    /// Last known usage from the most recent stream
    last_usage: Option<TokenUsage>,

    /// Current theme + palette
    current_theme: egui::Theme,
    pal: Palette,

    /// Particle system
    particles: Particles,
    last_frame_time: Option<f64>,
}

impl ChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        let theme = cc.egui_ctx.system_theme().unwrap_or(egui::Theme::Dark);
        let pal = Palette::for_theme(theme);
        apply_visuals(&cc.egui_ctx, theme, &pal);

        let db = match Database::open() {
            Ok(db) => db,
            Err(e) => {
                error!("Failed to open database: {e:#}");
                panic!("Cannot open database: {e:#}");
            }
        };

        let conversations = db.list_conversations().unwrap_or_default();

        let saved_key = db.get_config("api_key").ok().flatten();
        let saved_region = db
            .get_config("region")
            .ok()
            .flatten()
            .and_then(|r| REGIONS.iter().position(|&x| x == r))
            .unwrap_or(0);

        let screen = if let Some(ref key) = saved_key {
            if !key.is_empty() {
                std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", key);
                Screen::Chat
            } else {
                Screen::Credentials(CredentialForm::default())
            }
        } else {
            Screen::Credentials(CredentialForm::default())
        };

        let clipboard = arboard::Clipboard::new()
            .map_err(|e| warn!("clipboard unavailable: {e}"))
            .ok();

        Self {
            rt,
            db,
            screen,
            conversations,
            active_id: None,
            messages: Vec::new(),
            md_caches: HashMap::new(),
            streaming_md_cache: CommonMarkCache::default(),
            input: String::new(),
            stream_rx: None,
            is_streaming: false,
            last_error: None,
            scroll_to_bottom: false,
            model_idx: 0,
            region_idx: saved_region,
            show_system_prompt: false,
            clipboard,
            conv_usage: TokenUsage::default(),
            last_usage: None,
            current_theme: theme,
            pal,
            particles: Particles::new(40),
            last_frame_time: None,
        }
    }

    /// Check if OS theme changed and re-apply palette
    fn check_theme(&mut self, ctx: &egui::Context) {
        let theme = ctx.system_theme().unwrap_or(egui::Theme::Dark);
        if theme != self.current_theme {
            self.current_theme = theme;
            self.pal = Palette::for_theme(theme);
            apply_visuals(ctx, theme, &self.pal);
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn active_conversation(&self) -> Option<&Conversation> {
        self.active_id
            .as_ref()
            .and_then(|id| self.conversations.iter().find(|c| c.id == *id))
    }

    fn active_conversation_mut(&mut self) -> Option<&mut Conversation> {
        let id = self.active_id.clone()?;
        self.conversations.iter_mut().find(|c| c.id == id)
    }

    fn select_conversation(&mut self, id: &str) {
        self.active_id = Some(id.to_string());
        self.conv_usage = TokenUsage::default();
        self.last_usage = None;
        match self.db.list_messages(id) {
            Ok(msgs) => {
                self.messages = msgs;
                self.md_caches.clear();
                self.scroll_to_bottom = true;
            }
            Err(e) => {
                error!("failed to load messages: {e:#}");
                self.last_error = Some(format!("Failed to load messages: {e:#}"));
            }
        }
        let conv_data = self
            .active_conversation()
            .map(|c| (c.model_id.clone(), c.region.clone()));
        if let Some((model_id, region)) = conv_data {
            if let Some(idx) = MODELS.iter().position(|m| m.id == model_id) {
                self.model_idx = idx;
            }
            if let Some(idx) = REGIONS.iter().position(|r| *r == region) {
                self.region_idx = idx;
            }
        }
    }

    fn new_conversation(&mut self) {
        let model_id = MODELS[self.model_idx].id;
        let region = REGIONS[self.region_idx];
        let conv = Conversation::new("New Chat", model_id, region);
        if let Err(e) = self.db.upsert_conversation(&conv) {
            error!("failed to create conversation: {e:#}");
            self.last_error = Some(format!("DB error: {e:#}"));
            return;
        }
        let id = conv.id.clone();
        self.conversations.insert(0, conv);
        self.select_conversation(&id);
    }

    fn delete_conversation(&mut self, id: &str) {
        if let Err(e) = self.db.delete_conversation(id) {
            error!("failed to delete conversation: {e:#}");
            self.last_error = Some(format!("DB error: {e:#}"));
            return;
        }
        self.conversations.retain(|c| c.id != id);
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
            self.messages.clear();
            self.md_caches.clear();
            self.conv_usage = TokenUsage::default();
            self.last_usage = None;
        }
    }

    fn send_message(&mut self, ctx: &egui::Context) {
        let text = self.input.trim().to_string();
        if text.is_empty() { return; }
        let conv_id = match &self.active_id {
            Some(id) => id.clone(),
            None => {
                self.new_conversation();
                match &self.active_id { Some(id) => id.clone(), None => return }
            }
        };

        let user_msg = ChatMessage::new(&conv_id, Role::User, &text);
        if let Err(e) = self.db.insert_message(&user_msg) {
            error!("failed to insert user message: {e:#}");
            self.last_error = Some(format!("DB error: {e:#}"));
            return;
        }
        self.messages.push(user_msg);
        self.input.clear();

        if self.messages.len() == 1 {
            let title: String = text.chars().take(50).collect();
            if let Some(conv) = self.active_conversation_mut() {
                conv.title = title;
                conv.updated_at = chrono::Utc::now();
            }
            if let Some(conv) = self.active_conversation() {
                let _ = self.db.upsert_conversation(conv);
            }
        }

        let assistant_msg = ChatMessage::new(&conv_id, Role::Assistant, "");
        if let Err(e) = self.db.insert_message(&assistant_msg) {
            error!("failed to insert assistant message: {e:#}");
        }
        self.messages.push(assistant_msg);

        let history: Vec<(String, String)> = self.messages.iter()
            .filter(|m| !m.content.is_empty())
            .map(|m| (m.role.as_str().to_string(), m.content.clone()))
            .collect();

        let conv_info = self.active_conversation()
            .map(|c| (c.model_id.clone(), c.region.clone(), c.system_prompt.clone()));
        let (model_id, region, system_prompt) = match conv_info {
            Some(t) => t, None => return,
        };

        self.streaming_md_cache = CommonMarkCache::default();
        let rx = bedrock::spawn_stream(&self.rt, ctx.clone(), model_id, region, system_prompt, history);
        self.stream_rx = Some(rx);
        self.is_streaming = true;
        self.scroll_to_bottom = true;
    }

    fn poll_stream(&mut self) {
        let rx = match &mut self.stream_rx { Some(rx) => rx, None => return };

        loop {
            match rx.try_recv() {
                Ok(StreamToken::Delta(text)) => {
                    if let Some(msg) = self.messages.last_mut() {
                        msg.append_token(&text);
                        self.scroll_to_bottom = true;
                    }
                }
                Ok(StreamToken::Done(usage)) => {
                    info!("stream completed");
                    if let Some(u) = usage {
                        self.conv_usage.input_tokens += u.input_tokens;
                        self.conv_usage.output_tokens += u.output_tokens;
                        self.conv_usage.total_tokens += u.total_tokens;
                        self.last_usage = Some(u);
                    }
                    self.finish_stream();
                    break;
                }
                Ok(StreamToken::Error(e)) => {
                    self.last_error = Some(e);
                    self.finish_stream();
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    warn!("stream channel disconnected");
                    self.finish_stream();
                    break;
                }
            }
        }
    }

    fn finish_stream(&mut self) {
        self.is_streaming = false;
        self.stream_rx = None;

        if let Some(msg) = self.messages.last() {
            if msg.role == Role::Assistant {
                if let Err(e) = self.db.update_message_content(&msg.id, &msg.content) {
                    error!("failed to update message content: {e:#}");
                }
            }
        }

        if let Some(conv) = self.active_conversation_mut() {
            conv.updated_at = chrono::Utc::now();
        }
        if let Some(conv) = self.active_conversation() {
            let _ = self.db.upsert_conversation(conv);
        }
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        if let Some(ref mut cb) = self.clipboard {
            if let Err(e) = cb.set_text(text) { warn!("clipboard copy failed: {e}"); }
        }
    }

    // ── File menu bar ──────────────────────────────────────────────────

    fn render_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Change API Key...").clicked() {
                    self.screen = Screen::Credentials(CredentialForm {
                        api_key: String::new(),
                        region_idx: self.region_idx,
                    });
                    ui.close();
                }
                ui.separator();
                if ui.button("New Chat").clicked() {
                    self.new_conversation();
                    ui.close();
                }
            });
        });
    }

    // ── Credential modal ───────────────────────────────────────────────

    fn render_credentials_modal(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, pal.bg_base);

        // Draw particles on the credential screen too
        let now = ui.input(|i| i.time);
        let dt = self.last_frame_time.map_or(0.016, |t| (now - t) as f32).min(0.1);
        self.last_frame_time = Some(now);
        self.particles.update_and_draw(ui.painter(), rect, pal.accent, dt);

        ui.vertical_centered(|ui| {
            ui.add_space(rect.height() * 0.28);

            egui::Frame::new()
                .inner_margin(egui::Margin::same(32))
                .corner_radius(16.0)
                .fill(pal.bg_modal)
                .stroke(egui::Stroke::new(1.0, pal.border))
                .show(ui, |ui| {
                    ui.set_width(400.0);
                    ui.colored_label(pal.text_primary, egui::RichText::new("Bedrock Chat").size(24.0).strong());
                    ui.add_space(6.0);
                    ui.colored_label(pal.text_secondary, "Paste your Bedrock API key, or skip to use\nyour existing AWS config.");
                    ui.add_space(16.0);

                    let Screen::Credentials(form) = &mut self.screen else { return; };

                    ui.colored_label(pal.text_secondary, "API Key");
                    ui.add_space(2.0);
                    ui.add(egui::TextEdit::singleline(&mut form.api_key)
                        .desired_width(f32::INFINITY).password(true).hint_text("Paste Bedrock API key..."));
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.colored_label(pal.text_secondary, "Region");
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("cred_region")
                            .selected_text(REGIONS[form.region_idx])
                            .show_ui(ui, |ui| {
                                for (i, region) in REGIONS.iter().enumerate() {
                                    ui.selectable_value(&mut form.region_idx, i, *region);
                                }
                            });
                    });
                    ui.add_space(20.0);

                    ui.horizontal(|ui| {
                        let Screen::Credentials(form) = &self.screen else { return; };
                        let has_key = !form.api_key.trim().is_empty();

                        if ui.add_enabled(has_key, egui::Button::new(
                            egui::RichText::new("Connect").color(if has_key { pal.bg_base } else { pal.text_muted })
                        ).fill(if has_key { pal.accent } else { pal.bg_input }).corner_radius(8.0).min_size(egui::vec2(90.0, 32.0)))
                        .clicked() {
                            let Screen::Credentials(form) = &self.screen else { return; };
                            let key = form.api_key.trim().to_string();
                            std::env::set_var("AWS_BEARER_TOKEN_BEDROCK", &key);
                            let _ = self.db.set_config("api_key", &key);
                            let _ = self.db.set_config("region", REGIONS[form.region_idx]);
                            self.region_idx = form.region_idx;
                            self.screen = Screen::Chat;
                        }

                        ui.add_space(8.0);

                        if ui.add(egui::Button::new(
                            egui::RichText::new("Skip").color(pal.text_secondary)
                        ).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::new(1.0, pal.border)).corner_radius(8.0).min_size(egui::vec2(70.0, 32.0)))
                        .clicked() {
                            let Screen::Credentials(form) = &self.screen else { return; };
                            let _ = self.db.set_config("region", REGIONS[form.region_idx]);
                            self.region_idx = form.region_idx;
                            self.screen = Screen::Chat;
                        }
                    });
                });
        });

        ui.ctx().request_repaint();
    }

    // ── Sidebar ────────────────────────────────────────────────────────

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        ui.painter().rect_filled(ui.max_rect(), 0.0, pal.bg_sidebar);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.colored_label(pal.text_primary, egui::RichText::new("Chats").size(16.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if ui.add(egui::Button::new(egui::RichText::new("+").size(16.0).color(pal.accent))
                    .fill(egui::Color32::TRANSPARENT).corner_radius(6.0).min_size(egui::vec2(28.0, 28.0)))
                .clicked() { self.new_conversation(); }
            });
        });
        ui.add_space(4.0);

        let rect = ui.available_rect_before_wrap();
        ui.painter().line_segment([rect.left_top(), egui::pos2(rect.right(), rect.top())], egui::Stroke::new(1.0, pal.border));
        ui.add_space(6.0);

        let active_id = self.active_id.clone();
        let mut to_select: Option<String> = None;
        let mut to_delete: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for conv in &self.conversations {
                let is_active = active_id.as_deref() == Some(&conv.id);
                let bg = if is_active { pal.selected } else { egui::Color32::TRANSPARENT };
                let title: String = if conv.title.chars().count() > 28 {
                    conv.title.chars().take(25).collect::<String>() + "..."
                } else { conv.title.clone() };

                egui::Frame::new().fill(bg).corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .outer_margin(egui::Margin::symmetric(4, 1))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let tc = if is_active { pal.text_primary } else { pal.text_secondary };
                            let resp = ui.add(egui::Label::new(egui::RichText::new(&title).color(tc).size(13.0))
                                .selectable(false).sense(egui::Sense::click()));
                            if resp.clicked() && !is_active { to_select = Some(conv.id.clone()); }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if is_active || ui.rect_contains_pointer(ui.max_rect()) {
                                    if ui.add(egui::Button::new(egui::RichText::new("x").color(pal.text_muted).size(12.0))
                                        .fill(egui::Color32::TRANSPARENT).min_size(egui::vec2(20.0, 20.0))).clicked() {
                                        to_delete = Some(conv.id.clone());
                                    }
                                }
                            });
                        });
                    });
            }
        });

        if let Some(id) = to_select { self.select_conversation(&id); }
        if let Some(id) = to_delete { self.delete_conversation(&id); }
    }

    // ── Chat pane ──────────────────────────────────────────────────────

    fn render_chat_pane(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let full_rect = ui.max_rect();
        ui.painter().rect_filled(full_rect, 0.0, pal.bg_base);

        // Particles behind everything
        let now = ui.input(|i| i.time);
        let dt = self.last_frame_time.map_or(0.016, |t| (now - t) as f32).min(0.1);
        self.last_frame_time = Some(now);
        self.particles.update_and_draw(ui.painter(), full_rect, pal.accent, dt);

        if self.active_id.is_none() {
            ui.centered_and_justified(|ui| {
                ui.colored_label(pal.text_muted, "Select or create a conversation");
            });
            return;
        }

        self.render_top_bar(ui);

        let rect = ui.available_rect_before_wrap();
        ui.painter().line_segment([rect.left_top(), egui::pos2(rect.right(), rect.top())], egui::Stroke::new(1.0, pal.border));
        ui.add_space(2.0);

        let input_area_height = 100.0;
        let avail = ui.available_height() - input_area_height;
        ui.allocate_ui(egui::vec2(ui.available_width(), avail.max(100.0)), |ui| {
            self.render_messages(ui);
        });

        if let Some(err) = self.last_error.clone() {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.colored_label(pal.error, &err);
                if ui.small_button("dismiss").clicked() { self.last_error = None; }
            });
        }

        self.render_input(ui);

        // Request repaint for particle animation
        ui.ctx().request_repaint();
    }

    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        egui::Frame::new().fill(pal.bg_topbar).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(pal.text_secondary, "Model");
                ui.add_space(2.0);
                let current_name = MODELS[self.model_idx].name;
                egui::ComboBox::from_id_salt("model_picker")
                    .selected_text(current_name)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        let mut last_provider = "";
                        for (i, m) in MODELS.iter().enumerate() {
                            if m.provider != last_provider {
                                if !last_provider.is_empty() { ui.separator(); }
                                ui.colored_label(pal.text_muted, egui::RichText::new(m.provider).size(11.0).strong());
                                last_provider = m.provider;
                            }
                            ui.selectable_value(&mut self.model_idx, i, m.name);
                        }
                    });

                ui.add_space(8.0);
                ui.colored_label(pal.text_secondary, "Region");
                ui.add_space(2.0);
                egui::ComboBox::from_id_salt("region_picker")
                    .selected_text(REGIONS[self.region_idx])
                    .show_ui(ui, |ui| {
                        for (i, region) in REGIONS.iter().enumerate() {
                            ui.selectable_value(&mut self.region_idx, i, *region);
                        }
                    });

                ui.add_space(8.0);
                if ui.selectable_label(self.show_system_prompt, "System Prompt").clicked() {
                    self.show_system_prompt = !self.show_system_prompt;
                }

                // Token usage display
                if self.conv_usage.total_tokens > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(pal.text_muted, egui::RichText::new(
                            format!("{}in / {}out", self.conv_usage.input_tokens, self.conv_usage.output_tokens)
                        ).size(11.0));
                    });
                }
            });
        });

        // Sync model/region to conversation
        let model_id = MODELS[self.model_idx].id.to_string();
        let region = REGIONS[self.region_idx].to_string();
        if let Some(conv) = self.active_conversation_mut() {
            if conv.model_id != model_id || conv.region != region {
                conv.model_id = model_id;
                conv.region = region;
            }
        }
        if let Some(conv) = self.active_conversation() {
            let _ = self.db.upsert_conversation(conv);
        }

        if self.show_system_prompt {
            egui::Frame::new().fill(pal.bg_topbar).inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
                let mut sys = self.active_conversation().map(|c| c.system_prompt.clone()).unwrap_or_default();
                let changed = ui.add(egui::TextEdit::multiline(&mut sys)
                    .hint_text("Enter system prompt...").desired_rows(2).desired_width(f32::INFINITY)).changed();
                if changed {
                    if let Some(conv) = self.active_conversation_mut() { conv.system_prompt = sys; }
                    if let Some(conv) = self.active_conversation() { let _ = self.db.upsert_conversation(conv); }
                }
            });
        }
    }

    fn render_messages(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let side_pad = (ui.available_width() * 0.04).clamp(12.0, 40.0);
            ui.add_space(8.0);
            let msg_count = self.messages.len();
            for i in 0..msg_count {
                let is_last = i == msg_count - 1;
                let is_streaming_msg = is_last && self.is_streaming;
                ui.horizontal(|ui| {
                    ui.add_space(side_pad);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.set_width(ui.available_width() - side_pad);
                        self.render_single_message(ui, i, is_streaming_msg);
                    });
                });
            }
            if self.scroll_to_bottom {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                self.scroll_to_bottom = false;
            }
        });
    }

    fn render_single_message(&mut self, ui: &mut egui::Ui, idx: usize, is_streaming: bool) {
        let pal = self.pal.clone();
        let role = self.messages[idx].role;
        let (role_label, role_color, bg_color) = match role {
            Role::User => ("You", pal.role_user, pal.bg_user_msg),
            Role::Assistant => ("Assistant", pal.role_assistant, pal.bg_assist_msg),
        };

        let content_empty = self.messages[idx].content.is_empty();
        let content_for_copy = if role == Role::Assistant && !content_empty {
            Some(self.messages[idx].content.clone())
        } else { None };

        egui::Frame::new().fill(bg_color).corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .outer_margin(egui::Margin::symmetric(0, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.colored_label(role_color, egui::RichText::new(role_label).size(12.5).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(ref text) = content_for_copy {
                            if ui.add(egui::Button::new(egui::RichText::new("Copy").size(11.0).color(pal.text_muted))
                                .fill(egui::Color32::TRANSPARENT).corner_radius(4.0)).clicked() {
                                self.copy_to_clipboard(text);
                            }
                        }
                        if is_streaming { ui.spinner(); }
                    });
                });
                ui.add_space(6.0);

                if content_empty && is_streaming {
                    ui.colored_label(pal.text_muted, "...");
                } else if !content_empty {
                    let content = self.messages[idx].content.clone();
                    if is_streaming {
                        CommonMarkViewer::new().show(ui, &mut self.streaming_md_cache, &content);
                    } else {
                        let msg_id = self.messages[idx].id.clone();
                        let version = self.messages[idx].version;
                        let entry = self.md_caches.entry(msg_id)
                            .or_insert_with(|| (version, CommonMarkCache::default()));
                        if entry.0 != version { *entry = (version, CommonMarkCache::default()); }
                        CommonMarkViewer::new().show(ui, &mut entry.1, &content);
                    }
                }
            });
    }

    fn render_input(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let send_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter);
        let send_shortcut_ctrl = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Enter);

        egui::Frame::new().fill(egui::Color32::TRANSPARENT).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
            egui::Frame::new().fill(pal.bg_input).corner_radius(12.0)
                .stroke(egui::Stroke::new(1.0, pal.border)).inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let desired_rows = (self.input.chars().filter(|c| *c == '\n').count() + 1).clamp(1, 8);
                        let response = ui.add_sized(
                            egui::vec2(ui.available_width() - 60.0, 0.0),
                            egui::TextEdit::multiline(&mut self.input)
                                .hint_text(egui::RichText::new("Message...").color(pal.text_muted))
                                .desired_rows(desired_rows).lock_focus(true).text_color(pal.text_primary),
                        );

                        let ctrl_enter = ui.input_mut(|i| i.consume_shortcut(&send_shortcut) || i.consume_shortcut(&send_shortcut_ctrl));
                        let can_send = !self.is_streaming && !self.input.trim().is_empty();
                        let btn_color = if can_send { pal.accent } else { pal.accent_dim };
                        let send_clicked = ui.add(egui::Button::new(
                            egui::RichText::new("Send").color(if can_send { pal.bg_base } else { pal.text_muted }).size(13.0)
                        ).fill(btn_color).corner_radius(8.0).min_size(egui::vec2(52.0, 30.0))).clicked();

                        if (ctrl_enter || send_clicked) && can_send {
                            let ctx = ui.ctx().clone();
                            self.send_message(&ctx);
                        }
                        if !self.is_streaming { response.request_focus(); }
                    });
                });
        });
    }
}

// ── Visuals ─────────────────────────────────────────────────────────────

fn apply_visuals(ctx: &egui::Context, theme: egui::Theme, pal: &Palette) {
    let mut visuals = theme.default_visuals();

    visuals.panel_fill = pal.bg_base;
    visuals.window_fill = pal.bg_base;
    visuals.extreme_bg_color = pal.bg_input;
    visuals.faint_bg_color = pal.bg_sidebar;

    visuals.widgets.noninteractive.bg_fill = pal.bg_input;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, pal.text_primary);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, pal.border);

    visuals.widgets.inactive.bg_fill = pal.bg_input;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, pal.text_secondary);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, pal.border);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.hovered.bg_fill = pal.hover;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, pal.text_primary);
    visuals.widgets.active.bg_fill = pal.selected;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, pal.text_primary);

    visuals.selection.bg_fill = pal.accent_dim;
    visuals.selection.stroke = egui::Stroke::new(1.0, pal.text_primary);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    ctx.set_global_style(style);
}

// ── eframe::App ─────────────────────────────────────────────────────────

impl eframe::App for ChatApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.pal.bg_base;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.check_theme(ui.ctx());

        match &self.screen {
            Screen::Credentials(_) => {
                self.render_credentials_modal(ui);
            }
            Screen::Chat => {
                self.poll_stream();
                self.render_menu_bar(ui);

                egui::Panel::left("sidebar").default_size(240.0).min_size(180.0)
                    .show_inside(ui, |ui| { self.render_sidebar(ui); });

                egui::CentralPanel::default().show_inside(ui, |ui| { self.render_chat_pane(ui); });
            }
        }
    }
}
