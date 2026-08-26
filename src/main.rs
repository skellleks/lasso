mod app;
mod diff;
mod gitio;
mod herdr;
mod highlight;
mod review;
mod transcript;
mod tree;
mod ui;

mod runtime;

use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

use app::{App, Mouse};
use gitio::DiffBase;
use runtime::{
    apply_action, open_pane, read_lines, refresh, repo_root_for, spawn_socket_listener, state_dir,
    to_char, AppEvent,
};
use ui::FileView;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--open-pane") {
        return open_pane();
    }
    run_tui()
}

fn run_tui() -> Result<()> {
    let standalone = std::env::var("HERDR_ENV").ok().as_deref() != Some("1");
    let mut app = App::new(&state_dir(), standalone);
    let mut base = DiffBase::Head;
    let mut file_view = FileView::default();

    let (tx, rx) = mpsc::channel::<AppEvent>();

    // keyboard
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            let ev = match event::read() {
                Ok(Event::Key(k)) => Some(AppEvent::Key(k)),
                Ok(Event::Mouse(m)) => {
                    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
                    let kind = match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => Some(Mouse::LeftClick),
                        MouseEventKind::ScrollUp if shift => Some(Mouse::WheelLeft),
                        MouseEventKind::ScrollDown if shift => Some(Mouse::WheelRight),
                        MouseEventKind::ScrollUp => Some(Mouse::WheelUp),
                        MouseEventKind::ScrollDown => Some(Mouse::WheelDown),
                        MouseEventKind::ScrollLeft => Some(Mouse::WheelLeft),
                        MouseEventKind::ScrollRight => Some(Mouse::WheelRight),
                        _ => None,
                    };
                    kind.map(|k| AppEvent::Mouse(k, m.column, m.row))
                }
                _ => None,
            };
            if let Some(ev) = ev {
                if tx.send(ev).is_err() {
                    return;
                }
            }
        });
    }
    // periodic refresh fallback
    {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            if tx.send(AppEvent::Tick).is_err() {
                return;
            }
        });
    }
    if !standalone {
        spawn_socket_listener(tx.clone());
    }

    // pin this window to the agent it was opened for, permanently
    if !standalone {
        let agents = herdr::agent_list().unwrap_or_default();
        let (pane, ws) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .map(|ctx| herdr::parse_plugin_context(&ctx))
            .unwrap_or((None, None));
        if let Some(pin) = herdr::resolve_pin(&agents, pane.as_deref(), ws.as_deref()) {
            app.pin(pin);
        }
        app.set_agents(agents);
    }
    refresh(&mut app, base, standalone);

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), event::EnableMouseCapture);
    let result = (|| -> Result<()> {
        loop {
            {
                let size = terminal.size()?;
                let r = ui::regions(ratatui::layout::Rect::new(0, 0, size.width, size.height));
                app.files_viewport = r.files.height.saturating_sub(2) as usize;
                app.diff_viewport = r.right.height.saturating_sub(2) as usize;
            }
            // keep the full content of the current diff file loaded so the
            // diff renders with the whole file as context
            if let Some(display) = app.current_diff_path() {
                let (root, rel) = match app.current_file_location() {
                    Some((Some(root), rel)) => (root.to_path_buf(), rel.to_string()),
                    Some((None, rel)) => (repo_root_for(&app, standalone), rel.to_string()),
                    None => (repo_root_for(&app, standalone), display.clone()),
                };
                let lines = read_lines(&root.join(&rel));
                app.set_diff_content(&display, lines);
            }
            terminal.draw(|f| ui::draw(f, &app, &file_view))?;
            let ev = rx.recv()?;
            match ev {
                AppEvent::Mouse(m, x, y) => {
                    let size = terminal.size()?;
                    let r = ui::regions(ratatui::layout::Rect::new(0, 0, size.width, size.height));
                    let action = app.handle_mouse(m, x, y, &r);
                    if apply_action(action, &mut app, &mut base, &mut file_view, standalone) {
                        break;
                    }
                }
                AppEvent::Tick | AppEvent::AgentsChanged => {
                    // don't clobber typing state with refreshes
                    if app.modal.is_none() {
                        refresh(&mut app, base, standalone);
                    }
                }
                AppEvent::Key(k) => {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                    let Some(ch) = to_char(k.code, app.modal.is_some()) else {
                        continue;
                    };
                    let action = app.handle_key(ch);
                    if apply_action(action, &mut app, &mut base, &mut file_view, standalone) {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    let _ = crossterm::execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    result
}
