use std::collections::HashMap;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::bedrock::{self, StreamToken};
use crate::db::Database;
use crate::message::{ChatMessage, Conversation, Role, MODELS, REGIONS};

// ── App state ──────────────────────────────────────────────────────────

pub struct ChatApp {
    /// Tokio runtime handle for spawning async work
    rt: tokio::runtime::Handle,

    /// SQLite database
    db: Database,

    /// All conversations (sidebar list)
    conversations: Vec<Conversation>,

    /// Currently selected conversation ID
    active_id: Option<String>,

    /// Messages for the active conversation
    messages: Vec<ChatMessage>,

    /// Per-message markdown cache keyed by message_id.
    /// Stores (version, cache) so we know when to re-parse.
    md_caches: HashMap<String, (u64, CommonMarkCache)>,

    /// Markdown cache for the currently-streaming message
    streaming_md_cache: CommonMarkCache,

    /// Input text for the compose box
    input: String,

    /// Active streaming receiver (None when idle)
    stream_rx: Option<mpsc::UnboundedReceiver<StreamToken>>,

    /// Whether we're currently waiting for a response
    is_streaming: bool,

    /// Error to display
    last_error: Option<String>,

    /// Scroll to bottom flag
    scroll_to_bottom: bool,

    /// Selected model index
    model_idx: usize,

    /// Selected region index
    region_idx: usize,

    /// Whether the system prompt editor is open
    show_system_prompt: bool,

    /// Clipboard context
    clipboard: Option<arboard::Clipboard>,
}

impl ChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        cc.egui_ctx.set_global_style(style);

        let db = match Database::open() {
            Ok(db) => db,
            Err(e) => {
                error!("Failed to open database: {e:#}");
                panic!("Cannot open database: {e:#}");
            }
        };

        let conversations = db.list_conversations().unwrap_or_default();

        let clipboard = arboard::Clipboard::new()
            .map_err(|e| warn!("clipboard unavailable: {e}"))
            .ok();

        Self {
            rt,
            db,
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
            region_idx: 0,
            show_system_prompt: false,
            clipboard,
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
        // Sync model/region from conversation — extract values first to avoid borrow conflicts
        let conv_data = self.active_conversation().map(|c| (c.model_id.clone(), c.region.clone()));
        if let Some((model_id, region)) = conv_data {
            if let Some(idx) = MODELS.iter().position(|(_, mid)| *mid == model_id) {
                self.model_idx = idx;
            }
            if let Some(idx) = REGIONS.iter().position(|r| *r == region) {
                self.region_idx = idx;
            }
        }
    }

    fn new_conversation(&mut self) {
        let model_id = MODELS[self.model_idx].1;
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
        }
    }

    fn send_message(&mut self, ctx: &egui::Context) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let conv_id = match &self.active_id {
            Some(id) => id.clone(),
            None => {
                self.new_conversation();
                match &self.active_id {
                    Some(id) => id.clone(),
                    None => return,
                }
            }
        };

        // Add user message
        let user_msg = ChatMessage::new(&conv_id, Role::User, &text);
        if let Err(e) = self.db.insert_message(&user_msg) {
            error!("failed to insert user message: {e:#}");
            self.last_error = Some(format!("DB error: {e:#}"));
            return;
        }
        self.messages.push(user_msg);
        self.input.clear();

        // Update conversation title from first message
        if self.messages.len() == 1 {
            let title: String = text.chars().take(50).collect();
            if let Some(conv) = self.active_conversation_mut() {
                conv.title = title;
                conv.updated_at = chrono::Utc::now();
            }
            // Persist outside the mutable borrow
            if let Some(conv) = self.active_conversation() {
                let _ = self.db.upsert_conversation(conv);
            }
        }

        // Create placeholder assistant message
        let assistant_msg = ChatMessage::new(&conv_id, Role::Assistant, "");
        if let Err(e) = self.db.insert_message(&assistant_msg) {
            error!("failed to insert assistant message: {e:#}");
        }
        self.messages.push(assistant_msg);

        // Build history for the API (exclude the empty assistant placeholder)
        let history: Vec<(String, String)> = self
            .messages
            .iter()
            .filter(|m| !m.content.is_empty())
            .map(|m| (m.role.as_str().to_string(), m.content.clone()))
            .collect();

        // Get model/region from conversation
        let conv_info = self
            .active_conversation()
            .map(|c| (c.model_id.clone(), c.region.clone(), c.system_prompt.clone()));
        let (model_id, region, system_prompt) = match conv_info {
            Some(t) => t,
            None => return,
        };

        self.streaming_md_cache = CommonMarkCache::default();
        let rx = bedrock::spawn_stream(
            &self.rt,
            ctx.clone(),
            model_id,
            region,
            system_prompt,
            history,
        );
        self.stream_rx = Some(rx);
        self.is_streaming = true;
        self.scroll_to_bottom = true;
    }

    /// Poll the streaming channel for new tokens (non-blocking)
    fn poll_stream(&mut self) {
        let rx = match &mut self.stream_rx {
            Some(rx) => rx,
            None => return,
        };

        // Drain all available tokens this frame
        loop {
            match rx.try_recv() {
                Ok(StreamToken::Delta(text)) => {
                    if let Some(msg) = self.messages.last_mut() {
                        msg.append_token(&text);
                        self.scroll_to_bottom = true;
                    }
                }
                Ok(StreamToken::Done) => {
                    info!("stream completed");
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

        // Persist final assistant message content
        if let Some(msg) = self.messages.last() {
            if msg.role == Role::Assistant {
                if let Err(e) = self.db.update_message_content(&msg.id, &msg.content) {
                    error!("failed to update message content: {e:#}");
                }
            }
        }

        // Update conversation timestamp — clone data to avoid borrow conflicts
        if let Some(conv) = self.active_conversation_mut() {
            conv.updated_at = chrono::Utc::now();
        }
        if let Some(conv) = self.active_conversation() {
            let _ = self.db.upsert_conversation(conv);
        }
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        if let Some(ref mut cb) = self.clipboard {
            if let Err(e) = cb.set_text(text) {
                warn!("clipboard copy failed: {e}");
            }
        }
    }

    // ── UI Rendering ───────────────────────────────────────────────────

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Chats");
            if ui.button("+ New").clicked() {
                self.new_conversation();
            }
        });
        ui.separator();

        let active_id = self.active_id.clone();
        let mut to_select: Option<String> = None;
        let mut to_delete: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for conv in &self.conversations {
                let is_active = active_id.as_deref() == Some(&conv.id);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(&conv.title).selected(is_active))
                        .clicked()
                        && !is_active
                    {
                        to_select = Some(conv.id.clone());
                    }
                    if ui.small_button("x").clicked() {
                        to_delete = Some(conv.id.clone());
                    }
                });
            }
        });

        if let Some(id) = to_select {
            self.select_conversation(&id);
        }
        if let Some(id) = to_delete {
            self.delete_conversation(&id);
        }
    }

    fn render_chat_pane(&mut self, ui: &mut egui::Ui) {
        if self.active_id.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select or create a conversation");
            });
            return;
        }

        // Top bar: model picker, region picker, system prompt toggle
        self.render_top_bar(ui);
        ui.separator();

        // Message list (takes remaining space minus input box)
        let input_area_height = 120.0;
        let avail = ui.available_height() - input_area_height;
        ui.allocate_ui(egui::vec2(ui.available_width(), avail.max(100.0)), |ui| {
            self.render_messages(ui);
        });

        ui.separator();

        // Error display
        if let Some(err) = self.last_error.clone() {
            ui.colored_label(egui::Color32::RED, &err);
            if ui.small_button("dismiss").clicked() {
                self.last_error = None;
            }
        }

        // Input box
        self.render_input(ui);
    }

    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Model picker
            ui.label("Model:");
            let current_model_name = MODELS[self.model_idx].0;
            egui::ComboBox::from_id_salt("model_picker")
                .selected_text(current_model_name)
                .show_ui(ui, |ui| {
                    for (i, (name, _)) in MODELS.iter().enumerate() {
                        ui.selectable_value(&mut self.model_idx, i, *name);
                    }
                });

            // Region picker
            ui.label("Region:");
            let current_region = REGIONS[self.region_idx];
            egui::ComboBox::from_id_salt("region_picker")
                .selected_text(current_region)
                .show_ui(ui, |ui| {
                    for (i, region) in REGIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.region_idx, i, *region);
                    }
                });

            // System prompt toggle
            if ui
                .selectable_label(self.show_system_prompt, "System Prompt")
                .clicked()
            {
                self.show_system_prompt = !self.show_system_prompt;
            }
        });

        // Sync model/region back to conversation after combo box interaction
        let model_id = MODELS[self.model_idx].1.to_string();
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

        // System prompt editor
        if self.show_system_prompt {
            let mut sys = self
                .active_conversation()
                .map(|c| c.system_prompt.clone())
                .unwrap_or_default();
            let changed = ui
                .add(
                    egui::TextEdit::multiline(&mut sys)
                        .hint_text("Enter system prompt...")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            if changed {
                if let Some(conv) = self.active_conversation_mut() {
                    conv.system_prompt = sys;
                }
                if let Some(conv) = self.active_conversation() {
                    let _ = self.db.upsert_conversation(conv);
                }
            }
        }
    }

    fn render_messages(&mut self, ui: &mut egui::Ui) {
        let scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true);

        scroll.show(ui, |ui| {
            ui.set_width(ui.available_width());

            let msg_count = self.messages.len();
            for i in 0..msg_count {
                let is_last = i == msg_count - 1;
                let is_streaming_msg = is_last && self.is_streaming;
                self.render_single_message(ui, i, is_streaming_msg);
            }

            if self.scroll_to_bottom {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                self.scroll_to_bottom = false;
            }
        });
    }

    fn render_single_message(&mut self, ui: &mut egui::Ui, idx: usize, is_streaming: bool) {
        let role = self.messages[idx].role;
        let role_label = match role {
            Role::User => "You",
            Role::Assistant => "Assistant",
        };

        let bg_color = match role {
            Role::User => egui::Color32::from_rgba_premultiplied(40, 40, 60, 255),
            Role::Assistant => egui::Color32::from_rgba_premultiplied(30, 30, 30, 255),
        };

        let content_empty = self.messages[idx].content.is_empty();
        let content_for_copy = if role == Role::Assistant && !content_empty {
            Some(self.messages[idx].content.clone())
        } else {
            None
        };

        egui::Frame::new()
            .fill(bg_color)
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(12))
            .outer_margin(egui::Margin::symmetric(0, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                ui.horizontal(|ui| {
                    ui.strong(role_label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(ref text) = content_for_copy {
                            if ui.small_button("Copy").clicked() {
                                self.copy_to_clipboard(text);
                            }
                        }
                        if is_streaming {
                            ui.spinner();
                        }
                    });
                });

                ui.add_space(4.0);

                if content_empty && is_streaming {
                    ui.label("...");
                } else if !content_empty {
                    // We need to get a reference to content that outlives the borrow of md_caches
                    let content = self.messages[idx].content.clone();
                    if is_streaming {
                        CommonMarkViewer::new()
                            .show(ui, &mut self.streaming_md_cache, &content);
                    } else {
                        let msg_id = self.messages[idx].id.clone();
                        let version = self.messages[idx].version;
                        let entry = self
                            .md_caches
                            .entry(msg_id)
                            .or_insert_with(|| (version, CommonMarkCache::default()));
                        if entry.0 != version {
                            *entry = (version, CommonMarkCache::default());
                        }
                        CommonMarkViewer::new()
                            .show(ui, &mut entry.1, &content);
                    }
                }
            });
    }

    fn render_input(&mut self, ui: &mut egui::Ui) {
        // Ctrl+Enter or Cmd+Enter to send
        let send_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter);
        let send_shortcut_ctrl =
            egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Enter);

        ui.horizontal_top(|ui| {
            let desired_rows = {
                let line_count = self.input.chars().filter(|c| *c == '\n').count() + 1;
                line_count.clamp(1, 8)
            };

            let response = ui.add_sized(
                egui::vec2(ui.available_width() - 70.0, 0.0),
                egui::TextEdit::multiline(&mut self.input)
                    .hint_text("Type a message... (Ctrl+Enter to send)")
                    .desired_rows(desired_rows)
                    .lock_focus(true),
            );

            let ctrl_enter_pressed = ui.input_mut(|i| {
                i.consume_shortcut(&send_shortcut) || i.consume_shortcut(&send_shortcut_ctrl)
            });

            let can_send = !self.is_streaming && !self.input.trim().is_empty();
            let send_clicked = ui
                .add_enabled(can_send, egui::Button::new("Send"))
                .clicked();

            if (ctrl_enter_pressed || send_clicked) && can_send {
                let ctx = ui.ctx().clone();
                self.send_message(&ctx);
            }

            // Keep focus on the text input
            if !self.is_streaming {
                response.request_focus();
            }
        });
    }
}

// ── eframe::App implementation ──────────────────────────────────────────

impl eframe::App for ChatApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Poll streaming tokens before rendering
        self.poll_stream();

        egui::Panel::left("sidebar")
            .default_size(220.0)
            .min_size(150.0)
            .show_inside(ui, |ui| {
                self.render_sidebar(ui);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_chat_pane(ui);
        });
    }
}
