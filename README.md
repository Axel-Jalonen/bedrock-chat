# Marmaro

Native macOS chat client for AWS Bedrock. Built with Rust and egui.

## Features

- 64+ models from 16 providers (Claude, Llama, DeepSeek, Mistral, Qwen, Nova, etc.)
- Streaming responses with token usage display
- Markdown rendering with syntax highlighting and tables
- LaTeX math rendering (inline and display) via ReX
- Chat history with full-text search (Cmd+K)
- System prompts per conversation
- Ephemeral mode (nothing saved to disk)
- Context compaction (summarize history to save tokens)
- Credentials stored in macOS Keychain

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Enter | Send message |
| Shift+Enter | New line |
| Cmd+K | Search chats |
| Esc | Close modal |
| Esc Esc | Interrupt generation |

## Build

```bash
cargo run --release
```

Requires Rust 1.75+ and macOS.

## Technical Notes

- Pure Rust, no Electron/WebView
- LaTeX rendering uses [a patched fork of ReX](https://github.com/Axel-Jalonen/rex-retex-patched) with fixes for modern Rust and additional commands (`\text`, `\operatorname`, `\boxed`, Cyrillic, etc.)
- Markdown parsing via pulldown-cmark with custom renderer for inline math
