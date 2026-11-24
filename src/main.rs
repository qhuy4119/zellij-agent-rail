mod model;
mod render;
mod status;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::model::Rail;

#[derive(Default)]
struct AttentionRail {
    state: Rail,
}

impl ZellijPlugin for AttentionRail {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        // Stay selectable until permissions are granted, otherwise the
        // permission prompt cannot be focused to approve it (lesson learned
        // from other sidebar plugins).
        set_selectable(true);
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            // Attention comes from global snapshots and pane deltas; local
            // pane/tab manifests also keep idle agents visible without polling.
            EventType::GlobalAttentionUpdate,
            EventType::PaneAttentionChanged,
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
            EventType::Mouse,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(status) => {
                if matches!(status, PermissionStatus::Granted) {
                    // The rail is glance-and-click; keep it out of focus
                    // cycling so j/k pane navigation skips it.
                    set_selectable(false);
                }
                false
            }
            Event::GlobalAttentionUpdate(global) => {
                self.state.set_global_attention(global);
                true
            }
            Event::PaneAttentionChanged(pane_id, attention) => {
                self.state.update_pane_attention(pane_id, attention);
                true
            }
            Event::TabUpdate(tabs) => {
                self.state.set_tabs(tabs);
                true
            }
            Event::PaneUpdate(panes) => {
                self.state.set_panes(panes);
                true
            }
            Event::Mouse(mouse) => self.state.handle_mouse(mouse),
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let out = render::render(&mut self.state, rows, cols);
        // No trailing newline: the render already fills exactly `rows`
        // lines, and an extra one would scroll the header out of view.
        print!("{out}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

register_plugin!(AttentionRail);
