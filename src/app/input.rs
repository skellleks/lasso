//! Input handling: keys, mouse, modal flow, submit.

use super::*;

impl App {
    /// Mouse input mapped through the current screen regions.
    pub fn handle_mouse(&mut self, m: Mouse, x: u16, y: u16, regions: &Regions) -> Action {
        if self.modal.is_some() {
            return Action::None;
        }
        let inside = |r: &ratatui::layout::Rect| {
            x > r.x && x < r.x + r.width.saturating_sub(1) && y > r.y && y < r.y + r.height.saturating_sub(1)
        };
        if inside(&regions.agents) {
            // pinned-agent header: not interactive
            Action::None
        } else if inside(&regions.files) {
            let len = self.files_tree_rows().len();
            let files_viewport = if self.files_viewport > 0 {
                self.files_viewport
            } else {
                regions.files.height.saturating_sub(2) as usize
            };
            // the same clamped top the renderer uses
            let top = self.files_scroll.min(len.saturating_sub(files_viewport.max(1)));
            let idx = top + (y - regions.files.y - 1) as usize;
            match m {
                Mouse::LeftClick => {
                    if idx >= len {
                        return Action::None;
                    }
                    self.selected_file = idx;
                    self.focus = Focus::Files;
                    self.click_tree_row(idx)
                }
                Mouse::WheelDown | Mouse::WheelUp => {
                    let viewport = if self.files_viewport > 0 {
                        self.files_viewport
                    } else {
                        regions.files.height.saturating_sub(2) as usize
                    };
                    let max = len.saturating_sub(viewport);
                    self.files_scroll = if m == Mouse::WheelDown {
                        (self.files_scroll + 3).min(max)
                    } else {
                        self.files_scroll.saturating_sub(3)
                    };
                    Action::None
                }
                Mouse::WheelLeft | Mouse::WheelRight => Action::None,
            }
        } else if inside(&regions.right) {
            if matches!(m, Mouse::WheelLeft | Mouse::WheelRight) {
                self.hscroll = if m == Mouse::WheelRight {
                    self.hscroll.saturating_add(8)
                } else {
                    self.hscroll.saturating_sub(8)
                };
                return Action::None;
            }
            match self.right {
                RightPane::File => {
                    match m {
                        Mouse::WheelDown => self.fv_scroll = self.fv_scroll.saturating_add(3),
                        Mouse::WheelUp => self.fv_scroll = self.fv_scroll.saturating_sub(3),
                        _ => {
                            self.focus = Focus::Diff;
                            if !self.view_model.is_empty() {
                                let row = self.fv_scroll as usize + (y - regions.right.y - 1) as usize;
                                self.fv_cursor = row.min(self.view_model.len() - 1);
                            }
                        }
                    }
                    Action::None
                }
                RightPane::Diff => {
                    let len = self.rows().len();
                    if len == 0 {
                        return Action::None;
                    }
                    match m {
                        Mouse::WheelDown | Mouse::WheelUp => {
                            let viewport = if self.diff_viewport > 0 {
                                self.diff_viewport
                            } else {
                                regions.right.height.saturating_sub(2) as usize
                            };
                            let max = len.saturating_sub(viewport);
                            self.diff_scroll = if m == Mouse::WheelDown {
                                (self.diff_scroll + 3).min(max)
                            } else {
                                self.diff_scroll.saturating_sub(3)
                            };
                        }
                        _ => {
                            self.focus = Focus::Diff;
                            let viewport = if self.diff_viewport > 0 {
                                self.diff_viewport
                            } else {
                                regions.right.height.saturating_sub(2) as usize
                            };
                            // the same clamped top the renderer uses
                            let top = self.diff_scroll.min(len.saturating_sub(viewport.max(1)));
                            let row = top + (y - regions.right.y - 1) as usize;
                            self.cursor = row.min(len - 1);
                        }
                    }
                    Action::None
                }
            }
        } else {
            Action::None
        }
    }

    pub fn handle_key(&mut self, key: char) -> Action {
        if let Some(modal) = self.modal.take() {
            return self.handle_modal_key(modal, key);
        }
        match key {
            'q' => Action::Quit,
            'r' => Action::Refresh,
            'b' => Action::ToggleBase,
            'a' => {
                self.all_files_mode = !self.all_files_mode;
                self.selected_file = 0;
                self.files_scroll = 0;
                Action::Refresh
            }
            '\t' => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Diff,
                    _ => Focus::Files,
                };
                Action::None
            }
            'j' | 'k' if self.right == RightPane::File && self.focus != Focus::Files => {
                let len = self.view_model.len();
                self.fv_cursor = if key == 'j' {
                    (self.fv_cursor + 1).min(len.saturating_sub(1))
                } else {
                    self.fv_cursor.saturating_sub(1)
                };
                // keep the cursor visible
                if self.diff_viewport > 0 {
                    if self.fv_cursor < self.fv_scroll as usize {
                        self.fv_scroll = self.fv_cursor as u16;
                    } else if self.fv_cursor >= self.fv_scroll as usize + self.diff_viewport {
                        self.fv_scroll = (self.fv_cursor + 1 - self.diff_viewport) as u16;
                    }
                }
                Action::None
            }
            'j' | 'k' => {
                self.navigate(key == 'j');
                Action::None
            }
            'h' | 'l' => {
                self.hscroll = if key == 'l' {
                    self.hscroll.saturating_add(8)
                } else {
                    self.hscroll.saturating_sub(8)
                };
                Action::None
            }
            'd' => {
                self.right = RightPane::Diff;
                Action::None
            }
            '\n' if self.focus == Focus::Files => self.activate_tree_row(self.selected_file),
            'c' if !self.standalone && (self.focus == Focus::Diff || self.right == RightPane::File) => {
                if self.current_anchor().is_some() {
                    self.modal = Some(Modal::Input { buffer: String::new() });
                }
                Action::None
            }
            'x' if self.focus == Focus::Diff && !self.standalone => {
                let key = self.agent_key();
                let count = self.store.comments(&key).len();
                if count > 0 {
                    self.store.remove(&key, count - 1);
                }
                Action::None
            }
            'S' if !self.standalone => self.submit(false),
            _ => Action::None,
        }
    }

    fn handle_modal_key(&mut self, modal: Modal, key: char) -> Action {
        match modal {
            Modal::Input { mut buffer } => match key {
                '\n' => {
                    if !buffer.trim().is_empty() {
                        if let Some((path, side, line_no, quote)) = self.current_anchor() {
                            let agent_key = self.agent_key();
                            self.store.add(
                                &agent_key,
                                Comment { path, side, line_no, quote, text: buffer.trim().to_string() },
                            );
                        }
                    }
                    Action::None
                }
                '\u{1b}' => Action::None,
                '\u{8}' | '\u{7f}' => {
                    buffer.pop();
                    self.modal = Some(Modal::Input { buffer });
                    Action::None
                }
                ch => {
                    buffer.push(ch);
                    self.modal = Some(Modal::Input { buffer });
                    Action::None
                }
            },
            Modal::ConfirmSubmit => {
                if key == 'y' {
                    self.submit(true)
                } else {
                    Action::None
                }
            }
        }
    }

    fn submit(&mut self, confirmed: bool) -> Action {
        let key = self.agent_key();
        if self.store.comments(&key).is_empty() {
            return Action::None;
        }
        let Some(agent) = self.agent() else {
            return Action::None;
        };
        if !confirmed && agent.agent_status != "idle" {
            self.modal = Some(Modal::ConfirmSubmit);
            return Action::None;
        }
        Action::Submit {
            pane_id: agent.pane_id.clone(),
            text: compose_prompt(self.store.comments(&key)),
        }
    }

    fn navigate(&mut self, down: bool) {
        let step = |v: usize, len: usize| -> usize {
            if down {
                (v + 1).min(len.saturating_sub(1))
            } else {
                v.saturating_sub(1)
            }
        };
        match self.focus {
            Focus::Files => {
                let len = self.files_tree_rows().len();
                self.selected_file = step(self.selected_file, len);
                self.hscroll = 0;
                self.sync_diff_file();
                // keep the selection visible
                if self.files_viewport > 0 {
                    if self.selected_file < self.files_scroll {
                        self.files_scroll = self.selected_file;
                    } else if self.selected_file >= self.files_scroll + self.files_viewport {
                        self.files_scroll = self.selected_file + 1 - self.files_viewport;
                    }
                }
            }
            Focus::Diff => {
                self.cursor = step(self.cursor, self.rows().len());
                // keep the cursor visible
                if self.diff_viewport > 0 {
                    if self.cursor < self.diff_scroll {
                        self.diff_scroll = self.cursor;
                    } else if self.cursor >= self.diff_scroll + self.diff_viewport {
                        self.diff_scroll = self.cursor + 1 - self.diff_viewport;
                    }
                }
            }
        }
    }
}
