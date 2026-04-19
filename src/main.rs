mod app;
mod bedrock;
mod db;
mod message;

use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let rt_handle = rt.handle().clone();

    // ── Native macOS menu bar via muda ──────────────────────────────
    let menu = muda::Menu::new();

    let app_menu = muda::Submenu::new("Bedrock Chat", true);
    let _ = app_menu.append(&muda::PredefinedMenuItem::about(None, None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = app_menu.append(&muda::PredefinedMenuItem::quit(None));
    let _ = menu.append(&app_menu);

    let file_menu = muda::Submenu::new("File", true);
    let change_key_item = muda::MenuItem::new(
        "Change API Key...",
        true,
        Some(muda::accelerator::Accelerator::new(
            Some(muda::accelerator::Modifiers::META),
            muda::accelerator::Code::Comma,
        )),
    );
    let new_chat_item = muda::MenuItem::new(
        "New Chat",
        true,
        Some(muda::accelerator::Accelerator::new(
            Some(muda::accelerator::Modifiers::META),
            muda::accelerator::Code::KeyN,
        )),
    );
    let search_item = muda::MenuItem::new(
        "Search Chats...",
        true,
        Some(muda::accelerator::Accelerator::new(
            Some(muda::accelerator::Modifiers::META),
            muda::accelerator::Code::KeyK,
        )),
    );
    let compact_item = muda::MenuItem::new("Compact Context", true, None);
    let ephemeral_item = muda::CheckMenuItem::new("Ephemeral Mode", true, false, None);
    let _ = file_menu.append(&change_key_item);
    let _ = file_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = file_menu.append(&new_chat_item);
    let _ = file_menu.append(&search_item);
    let _ = file_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = file_menu.append(&compact_item);
    let _ = file_menu.append(&ephemeral_item);
    let _ = file_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = file_menu.append(&muda::PredefinedMenuItem::close_window(None));
    let _ = menu.append(&file_menu);

    let edit_menu = muda::Submenu::new("Edit", true);
    let _ = edit_menu.append(&muda::PredefinedMenuItem::undo(None));
    let _ = edit_menu.append(&muda::PredefinedMenuItem::redo(None));
    let _ = edit_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = edit_menu.append(&muda::PredefinedMenuItem::cut(None));
    let _ = edit_menu.append(&muda::PredefinedMenuItem::copy(None));
    let _ = edit_menu.append(&muda::PredefinedMenuItem::paste(None));
    let _ = edit_menu.append(&muda::PredefinedMenuItem::select_all(None));
    let _ = menu.append(&edit_menu);

    let window_menu = muda::Submenu::new("Window", true);
    let _ = window_menu.append(&muda::PredefinedMenuItem::minimize(None));
    let _ = window_menu.append(&muda::PredefinedMenuItem::fullscreen(None));
    let _ = menu.append(&window_menu);

    menu.init_for_nsapp();

    // Capture menu item IDs for event handling in the app
    let menu_ids = app::MenuIds {
        change_key: change_key_item.id().clone(),
        new_chat: new_chat_item.id().clone(),
        search: search_item.id().clone(),
        compact: compact_item.id().clone(),
        ephemeral: ephemeral_item.id().clone(),
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Bedrock Chat")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([600.0, 400.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(true)
            .with_title_shown(true),
        ..Default::default()
    };

    eframe::run_native(
        "Bedrock Chat",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::ChatApp::new(cc, rt_handle, menu_ids)))),
    )
}
