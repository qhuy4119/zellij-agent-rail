//! State for the attention rail.
//!
//! Everything here is push-fed by Zellij events:
//! - `GlobalAttentionUpdate` gives attention items and counts across running
//!   sessions (the same data the built-in inbox shows), including remembered
//!   statuses.
//! - `TabUpdate` and `PaneUpdate` provide the current session's full pane
//!   inventory, including agents that are currently idle.
//! - `PaneAttentionChanged` refreshes a single pane's attention in place.
//!
//! No polling, no subprocesses, no blocking host calls on any event path.
//!
//! Agent identity outlives a single snapshot: a pane that reported its own
//! status (OSC 7501) is remembered in `agent_cache` keyed by
//! `(session, pane_id)`, so an agent that finishes — current attention back
//! to Idle, completion kept in `attention.remembered` — still shows as that
//! agent with its done state instead of vanishing from the list.

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

/// A clickable region produced during render, consumed on the next mouse
/// event. `row` is the pane-content row the hit occupies.
#[derive(Clone, Debug)]
pub enum Hit {
    /// Another running session (its rows in the sessions section).
    Session { name: String },
    /// An attention item (agent or urgent pane) in any session.
    Item { session: String, pane_id: PaneId },
}

/// One rendered entry in the agents section: an agent, or a pane whose
/// attention is worth acting on.
#[derive(Clone, Debug)]
pub struct Entry {
    pub session: String,
    pub pane_id: PaneId,
    /// True when the pane is (or was) a program reporting its own status.
    pub is_agent: bool,
    /// The reporting program's name, when known (now or from the cache).
    pub app: Option<String>,
    /// The pane's title, as a fallback identity.
    pub title: String,
    pub tab_name: String,
    /// The status to display: current, or a remembered completion.
    pub status: AttentionStatus,
    /// Full attention, for hints (blocked kind, progress, exit code).
    pub attention: PaneAttention,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollTarget {
    Sessions,
    #[default]
    Agents,
}

#[derive(Default)]
pub struct Rail {
    /// Current session name, from the global attention snapshot.
    pub current_session: String,
    /// Every attention item of every running session, with its session name.
    pub items: Vec<(String, AttentionItem)>,
    /// Every running session with its aggregate attention (including ours).
    pub sessions: Vec<GlobalSessionAttention>,
    /// Current-session tabs and panes. PaneUpdate includes idle panes omitted
    /// from the global attention snapshot.
    pub tabs: BTreeMap<usize, (usize, String)>,
    pub panes: PaneManifest,
    /// Aggregate attention counts across all sessions.
    pub counts: AttentionCounts,
    /// Panes that have ever reported their own status, keyed by
    /// (session, pane id): the app name they reported. Reconciled against
    /// attention items and the current session's pane inventory.
    pub agent_cache: BTreeMap<(String, PaneId), String>,

    /// Clickable rows, rebuilt on every render: (row, x_start, x_end, hit).
    /// `row` is pane-content-relative, matching mouse events.
    pub hits: Vec<(usize, usize, usize, Hit)>,
    /// Independent list offsets, measured in whole entries.
    pub sessions_scroll: usize,
    pub agents_scroll: usize,
    /// Mouse wheel scrolls the section most recently hovered.
    pub scroll_target: ScrollTarget,
    /// Top row of the lower agents panel, set during render.
    pub agents_panel_start: usize,
}

impl Rail {
    pub fn set_global_attention(&mut self, global: GlobalAttention) {
        if self.current_session != global.current_session {
            // PaneUpdate/TabUpdate describe only the active session; never
            // briefly attribute the previous session's panes to the new one.
            self.panes = PaneManifest::default();
            self.tabs.clear();
        }
        self.current_session = global.current_session.clone();
        self.sessions = global.sessions.clone();
        self.counts = global.counts;
        self.items = global
            .items
            .into_iter()
            .map(|item| (item.session.clone(), item.item))
            .collect();

        // Reconcile the agent cache: remember every current reporter, drop
        // panes that no longer exist, and retain local idle agents that are
        // absent from the attention snapshot.
        self.reconcile_agent_cache();
    }

    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>) {
        self.tabs = tabs
            .into_iter()
            .map(|tab| (tab.position, (tab.tab_id, tab.name)))
            .collect();
    }

    pub fn set_panes(&mut self, panes: PaneManifest) {
        self.panes = panes;
        for (id, app) in self.current_reporters() {
            self.agent_cache
                .insert((self.current_session.clone(), id), app);
        }
        self.reconcile_agent_cache();
    }

    pub fn update_pane_attention(&mut self, pane_id: PaneId, attention: PaneAttention) {
        for (session, item) in &mut self.items {
            if *session == self.current_session && item.pane_id == pane_id {
                item.attention = attention.clone();
            }
        }
        for pane in self.panes.panes.values_mut().flatten() {
            if pane_id_of(pane) == pane_id {
                pane.attention = attention.clone();
            }
        }
    }

    fn current_reporters(&self) -> Vec<(PaneId, String)> {
        self.panes
            .panes
            .values()
            .flatten()
            .filter_map(|pane| pane_reporter_app(pane).map(|app| (pane_id_of(pane), app)))
            .collect()
    }

    fn reconcile_agent_cache(&mut self) {
        let mut live: BTreeMap<(String, PaneId), String> = BTreeMap::new();
        for (session, item) in &self.items {
            if let Some(app) = reporter_app(&item.attention) {
                live.insert((session.clone(), item.pane_id), app);
            }
        }
        for (id, app) in self.current_reporters() {
            live.insert((self.current_session.clone(), id), app);
        }
        for (key, app) in &self.agent_cache {
            let still_exists = self
                .items
                .iter()
                .any(|(session, item)| session == &key.0 && item.pane_id == key.1)
                || (key.0 == self.current_session
                    && self
                        .panes
                        .panes
                        .values()
                        .flatten()
                        .any(|pane| pane_id_of(pane) == key.1));
            if still_exists && !live.contains_key(key) {
                live.insert(key.clone(), app.clone());
            }
        }
        self.agent_cache = live;
    }

    /// Sidebar entries across all sessions: agents (always, any state, and
    /// remembered completions), plus non-agent panes whose attention is
    /// worth acting on (needs you / failed / done). Sorted by urgency.
    pub fn sidebar_entries(&self) -> Vec<Entry> {
        // The global snapshot intentionally centers on attention. Merge in
        // current-session panes so agents reporting Idle remain discoverable.
        let mut items = self.items.clone();
        for (position, panes) in &self.panes.panes {
            let (tab_id, tab_name) = self
                .tabs
                .get(position)
                .cloned()
                .unwrap_or((*position, format!("Tab #{}", position + 1)));
            for pane in panes {
                let id = pane_id_of(pane);
                if items
                    .iter()
                    .any(|(session, item)| session == &self.current_session && item.pane_id == id)
                {
                    continue;
                }
                let app = pane_reporter_app(pane).or_else(|| {
                    self.agent_cache
                        .get(&(self.current_session.clone(), id))
                        .cloned()
                });
                let status = display_status(
                    &pane.attention.status,
                    pane.attention.remembered.as_ref().map(|r| r.status),
                );
                let urgent = matches!(
                    status,
                    AttentionStatus::NeedsYou | AttentionStatus::Failed | AttentionStatus::Done
                );
                if app.is_some() || urgent {
                    items.push((
                        self.current_session.clone(),
                        AttentionItem {
                            pane_id: id,
                            tab_id,
                            tab_name: tab_name.clone(),
                            pane_title: pane.title.clone(),
                            attention: pane.attention.clone(),
                        },
                    ));
                }
            }
        }

        let mut entries: Vec<Entry> = Vec::new();
        for (session, item) in &items {
            let cached_app = self.agent_cache.get(&(session.clone(), item.pane_id));
            let reporting_app = reporter_app(&item.attention);
            let app = reporting_app.or_else(|| cached_app.cloned());
            let is_agent = app.is_some();

            // The remembered completion supplements the live state, never
            // overrides it: an agent that finished earlier and is now
            // working or blocked must show its live state, while a pane
            // that went idle keeps showing what was worth remembering.
            let status = display_status(
                &item.attention.status,
                item.attention.remembered.as_ref().map(|r| r.status.clone()),
            );

            let urgent = matches!(
                status,
                AttentionStatus::NeedsYou | AttentionStatus::Failed | AttentionStatus::Done
            );
            if !is_agent && !urgent {
                continue;
            }

            entries.push(Entry {
                session: session.clone(),
                pane_id: item.pane_id,
                is_agent,
                app,
                title: item.pane_title.clone(),
                tab_name: item.tab_name.clone(),
                status,
                attention: item.attention.clone(),
            });
        }
        entries.sort_by(|a, b| {
            crate::status::status_rank(&b.status)
                .cmp(&crate::status::status_rank(&a.status))
                .then_with(|| b.is_agent.cmp(&a.is_agent))
                .then_with(|| a.app.cmp(&b.app).then_with(|| a.title.cmp(&b.title)))
        });
        entries
    }

    pub fn handle_mouse(&mut self, mouse: Mouse) -> bool {
        match mouse {
            Mouse::Hover(line, _) => {
                self.set_scroll_target(line);
                false
            }
            Mouse::LeftClick(line, column) => {
                self.set_scroll_target(line);
                let row = if line < 0 { 0 } else { line as usize };
                if let Some(hit) = self.hit_at(row, column).cloned() {
                    self.activate(&hit);
                    true
                } else {
                    false
                }
            }
            Mouse::ScrollUp(count) => {
                self.scroll_target_by(-(count.max(1) as isize));
                true
            }
            Mouse::ScrollDown(count) => {
                self.scroll_target_by(count.max(1) as isize);
                true
            }
            _ => false,
        }
    }

    fn set_scroll_target(&mut self, line: isize) {
        let row = line.max(0) as usize;
        if row >= self.agents_panel_start {
            self.scroll_target = ScrollTarget::Agents;
        } else if row >= 3 {
            self.scroll_target = ScrollTarget::Sessions;
        }
    }

    fn scroll_target_by(&mut self, delta: isize) {
        let scroll = match self.scroll_target {
            ScrollTarget::Sessions => &mut self.sessions_scroll,
            ScrollTarget::Agents => &mut self.agents_scroll,
        };
        if delta < 0 {
            *scroll = scroll.saturating_sub(delta.unsigned_abs());
        } else {
            *scroll = scroll.saturating_add(delta as usize);
        }
    }

    fn hit_at(&self, row: usize, column: usize) -> Option<&Hit> {
        self.hits
            .iter()
            .find(|(y, x0, x1, _)| *y == row && column >= *x0 && column < *x1)
            .map(|(_, _, _, hit)| hit)
    }

    fn activate(&mut self, hit: &Hit) {
        match hit {
            Hit::Item { session, pane_id } => {
                mark_session_pane_seen(session, *pane_id);
                if session == &self.current_session {
                    if let PaneId::Terminal(id) = pane_id {
                        focus_terminal_pane(*id, false, false);
                    }
                } else {
                    focus_session_pane(session, *pane_id);
                }
            }
            Hit::Session { name } => {
                switch_session(Some(name));
            }
        }
    }
}

fn pane_id_of(pane: &PaneInfo) -> PaneId {
    if pane.is_plugin {
        PaneId::Plugin(pane.id)
    } else {
        PaneId::Terminal(pane.id)
    }
}

/// The app name a current pane reports through PaneUpdate, if any.
fn pane_reporter_app(pane: &PaneInfo) -> Option<String> {
    pane.program_status
        .iter()
        .find(|record| record.is_root())
        .and_then(|record| record.app.clone())
        .or_else(|| {
            pane.program_status
                .iter()
                .find_map(|record| record.app.clone())
        })
        .or_else(|| reporter_app(&pane.attention))
        .or_else(|| (!pane.program_status.is_empty()).then(|| "agent".to_string()))
}

/// The app name a pane is currently reporting (OSC 7501), if any.
fn reporter_app(attention: &PaneAttention) -> Option<String> {
    if matches!(attention.source, Some(AttentionSource::ProgramStatus))
        || attention.detail.app.is_some()
    {
        Some(
            attention
                .detail
                .app
                .clone()
                .unwrap_or_else(|| "agent".into()),
        )
    } else {
        None
    }
}

/// Priority between the pane's live attention status and its remembered
/// (recently reported) status:
/// 1. A live needs-you/failed state always wins.
/// 2. A remembered blocked/failed record (e.g. a blocked subagent while the
///    root task keeps working) outranks live working/done.
/// 3. Any other live activity (working/done) wins over a stale remembered
///    completion.
/// 4. An idle/quiet pane shows what was worth remembering, if anything —
///    this is what keeps completed agents listed.
fn display_status(
    current: &AttentionStatus,
    remembered: Option<AttentionStatus>,
) -> AttentionStatus {
    if matches!(current, AttentionStatus::NeedsYou | AttentionStatus::Failed) {
        return current.clone();
    }
    if matches!(
        remembered,
        Some(AttentionStatus::NeedsYou) | Some(AttentionStatus::Failed)
    ) {
        return remembered.expect("checked above");
    }
    if crate::status::status_rank(current) >= crate::status::status_rank(&AttentionStatus::Working)
    {
        return current.clone();
    }
    remembered.unwrap_or_else(|| current.clone())
}
