//! Rendering for the attention rail.
//!
//! Structure follows herdr's sidebar: sessions occupy the upper area and the
//! two-line agent list is anchored in a dedicated lower area.
//!
//! ```text
//! ╭─ zellij ──────────────────╮
//! │ ◉ 2 need · ✗ 1 fail       │
//! │                           │
//! │ Sessions                  │
//! │ ▸ unflappable-duck current│
//! │   · 2 need · 1 run        │
//! │                           │
//! │       (session area)      │
//! │───────────────────────────│
//! │ Agents (2)                 │
//! │ ▸ unflappable-duck · #1   │
//! │   pi · review api         │
//! │ ✓ polished-foxglove · #2  │
//! │   claude                  │
//! │       (agent area)        │
//! ╰─ click: jump · scroll ────╯
//! ```
//!
//! Status glyphs use the terminal palette; the current-session row gets a
//! low-intensity charcoal surface instead of a high-contrast inverse. Every
//! row registers a full-width hit region at its pane-content row.

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::model::{Hit, Rail};
use crate::status::*;

/// Rows rendered before the body (top border, counts, spacer). Mouse events
/// are pane-content-relative, so hit rows must account for these.
const BODY_OFFSET: usize = 3;
const SELECTED_ROW_STYLE: &str = "\u{1b}[48;2;38;38;44m";

/// One styled piece of a line. `text` is plain; `style` is prepended and
/// RESET appended only if the piece is actually rendered.
struct Seg {
    text: String,
    style: &'static str,
}

/// A logical list entry may occupy more than one row; scrolling never splits it.
struct PanelEntry {
    rows: Vec<(String, Option<Hit>)>,
}

fn seg(text: impl Into<String>) -> Seg {
    Seg {
        text: text.into(),
        style: "",
    }
}

fn styled(text: impl Into<String>, style: &'static str) -> Seg {
    Seg {
        text: text.into(),
        style,
    }
}

fn seg_width(parts: &[Seg]) -> usize {
    parts
        .iter()
        .map(|s| graphemes::display_width(&s.text))
        .sum()
}

/// Compose segments into exactly `width` display columns: truncates the tail
/// (with an ellipsis on the last kept segment) and pads with spaces.
fn compose(parts: Vec<Seg>, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for part in &parts {
        let w = graphemes::display_width(&part.text);
        if used + w > width {
            let budget = width.saturating_sub(used);
            if budget > 0 {
                let cut = graphemes::truncate_end(&part.text, budget, "…").into_owned();
                if !cut.is_empty() {
                    out.push_str(part.style);
                    out.push_str(&cut);
                    out.push_str(RESET);
                    used += graphemes::display_width(&cut);
                }
            }
            break;
        }
        if !part.text.is_empty() {
            out.push_str(part.style);
            out.push_str(&part.text);
            out.push_str(RESET);
        }
        used += w;
    }
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

/// Compose with the last segment right-aligned instead of padded.
fn compose_lr(left: Vec<Seg>, right: Vec<Seg>, width: usize) -> String {
    let right_w = seg_width(&right);
    if right_w >= width {
        return compose(right, width);
    }
    let mut line = compose(left, width - right_w);
    for part in &right {
        if !part.text.is_empty() {
            line.push_str(part.style);
            line.push_str(&part.text);
            line.push_str(RESET);
        }
    }
    line
}

/// Highest priority of `status` and all the non-zero counts.
fn aggregate(status: &AttentionStatus, counts: &AttentionCounts) -> AttentionStatus {
    if counts.needs_you > 0 {
        return AttentionStatus::NeedsYou;
    }
    if counts.failed > 0 {
        return AttentionStatus::Failed;
    }
    if status_rank(status) >= status_rank(&AttentionStatus::Done) {
        return status.clone();
    }
    if counts.done > 0 {
        return AttentionStatus::Done;
    }
    if counts.working > 0 {
        return AttentionStatus::Working;
    }
    status.clone()
}

/// Compact Zellij's auto-generated tab names without losing the location.
fn tab_context(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("Tab #") {
        return (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
            .then(|| format!("#{rest}"));
    }
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("#{trimmed}"));
    }
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

// ---------------------------------------------------------------------------

pub fn render(rail: &mut Rail, rows: usize, cols: usize) -> String {
    rail.hits.clear();

    if rows < 9 || cols < 12 {
        return String::new();
    }

    let inner = cols.saturating_sub(2); // between the box borders
    let mut session_entries: Vec<PanelEntry> = Vec::new();

    // ---- SESSIONS (top panel) ----
    let current_attention = rail
        .sessions
        .iter()
        .find(|s| s.name == rail.current_session)
        .map(|s| s.attention.clone());

    let (glyph, color) = status_glyph(&match &current_attention {
        Some(a) => aggregate(&a.status, &a.counts),
        None => AttentionStatus::Idle,
    });
    let current = Row::new((glyph, color), rail.current_session.clone(), None)
        .style(BOLD)
        .right("current");
    let (line, _) = row_line(current, inner);
    let mut current_rows = vec![(selected_row(&line), None)];
    if let Some(attention) = &current_attention {
        let counts = session_counts_right(&attention.counts);
        if !counts.is_empty() {
            let counts_line = compose(vec![seg("   "), styled(format!("· {counts}"), DIM)], inner);
            current_rows.push((selected_row(&counts_line), None));
        }
    }
    session_entries.push(PanelEntry { rows: current_rows });

    // Each other Zellij session has a state row and dim secondary counts.
    for session in rail
        .sessions
        .iter()
        .filter(|s| s.name != rail.current_session)
    {
        let (glyph, color) = status_glyph(&aggregate(
            &session.attention.status,
            &session.attention.counts,
        ));
        let hit = Hit::Session {
            name: session.name.clone(),
        };
        let row = Row::new((glyph, color), session.name.clone(), Some(hit.clone())).style(BOLD);
        let (line, _) = row_line(row, inner);
        let mut rows = vec![(line, Some(hit.clone()))];
        let counts = session_counts_right(&session.attention.counts);
        if !counts.is_empty() {
            rows.push((
                compose(vec![seg("   "), styled(format!("· {counts}"), DIM)], inner),
                Some(hit),
            ));
        }
        session_entries.push(PanelEntry { rows });
    }

    // ---- AGENTS (bottom panel) ----
    let entries = rail.sidebar_entries();
    let known_agents = entries.iter().filter(|e| e.is_agent).count();
    let mut identity_counts: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for entry in &entries {
        let key = (
            entry_identity(entry),
            entry.session.clone(),
            entry.tab_name.clone(),
        );
        *identity_counts.entry(key).or_insert(0) += 1;
    }

    let mut agent_entries: Vec<PanelEntry> = Vec::new();
    if entries.is_empty() {
        agent_entries.push(PanelEntry {
            rows: vec![(
                compose(vec![seg("   "), styled("no agents reporting", DIM)], inner),
                None,
            )],
        });
    }

    for entry in &entries {
        let (glyph, color) = status_glyph(&entry.status);
        let hit = Hit::Item {
            session: entry.session.clone(),
            pane_id: entry.pane_id,
        };
        let identity = entry_identity(entry);
        let key = (
            identity.clone(),
            entry.session.clone(),
            entry.tab_name.clone(),
        );
        let duplicate = identity_counts.get(&key).copied().unwrap_or(0) > 1;
        let pane_id = match entry.pane_id {
            PaneId::Terminal(id) => id,
            PaneId::Plugin(id) => id,
        };
        let pane_tag = format!("p{pane_id}");
        let put_pane_tag_in_context = duplicate
            && 3 + graphemes::display_width(&identity) + 3 + graphemes::display_width(&pane_tag)
                > inner;
        let label_identity = if duplicate && !put_pane_tag_in_context {
            format!("{identity} · {pane_tag}")
        } else {
            identity
        };

        let text = sanitize_display_text(&entry.attention.text);
        let message = if !text.is_empty()
            && text != status_label(&entry.status)
            && text != status_label(&entry.attention.status)
        {
            text
        } else {
            attention_hint(&entry.attention)
        };
        let label = if message.is_empty() {
            label_identity
        } else {
            format!("{label_identity} · {message}")
        };

        let context = agent_context(entry, put_pane_tag_in_context, inner);
        let context_row = Row::new((glyph, color), context, Some(hit.clone())).style(DIM);
        let (context_line, _) = row_line(context_row, inner);
        let label_line = compose(vec![seg("   "), styled(label, BOLD)], inner);
        agent_entries.push(PanelEntry {
            rows: vec![(context_line, Some(hit.clone())), (label_line, Some(hit))],
        });
    }

    // Keep the same top/bottom balance as the reference sidebars. The agents
    // panel gets the lower 45%; the divider is part of the fixed body.
    let body_rows = rows.saturating_sub(BODY_OFFSET + 1); // top chrome + footer
    let (sessions_height, divider_rows, agents_height) = panel_heights(body_rows);
    let agents_top = BODY_OFFSET + sessions_height + divider_rows;
    rail.agents_panel_start = agents_top;

    let mut out = String::new();
    let mut hits = Vec::new();

    out.push_str(&boxed_top(cols, "zellij"));
    out.push('\n');

    let mut counts_segs = vec![seg(" ")];
    counts_segs.extend(counts_segments(&rail.counts));
    push_boxed_line(&mut out, &compose(counts_segs, inner), inner);
    push_boxed_line(&mut out, &" ".repeat(inner), inner);

    draw_panel(
        &mut out,
        &session_entries,
        "Sessions",
        None,
        &mut rail.sessions_scroll,
        BODY_OFFSET,
        sessions_height,
        inner,
        cols,
        &mut hits,
    );

    for _ in 0..divider_rows {
        let divider = compose(vec![styled("─".repeat(inner), DIM)], inner);
        push_boxed_line(&mut out, &divider, inner);
    }

    draw_panel(
        &mut out,
        &agent_entries,
        "Agents",
        Some(known_agents),
        &mut rail.agents_scroll,
        agents_top,
        agents_height,
        inner,
        cols,
        &mut hits,
    );

    rail.hits = hits;
    out.push_str(&boxed_bottom(cols, "click: jump · scroll"));
    out
}

fn panel_heights(body_rows: usize) -> (usize, usize, usize) {
    let divider = usize::from(body_rows >= 5);
    let panel_rows = body_rows.saturating_sub(divider);
    if panel_rows <= 3 {
        return (0, divider, panel_rows);
    }

    let sessions = ((panel_rows * 11 + 19) / 20).clamp(1, panel_rows - 3);
    (sessions, divider, panel_rows - sessions)
}

fn draw_panel(
    out: &mut String,
    entries: &[PanelEntry],
    title: &str,
    count: Option<usize>,
    scroll: &mut usize,
    top: usize,
    height: usize,
    inner: usize,
    cols: usize,
    hits: &mut Vec<(usize, usize, usize, Hit)>,
) {
    if height == 0 {
        return;
    }

    let header = section_header(title, count, inner);
    push_boxed_line(out, &header, inner);
    let content_height = height.saturating_sub(1);
    let max_scroll = max_entry_scroll(entries, content_height);
    *scroll = (*scroll).min(max_scroll);

    let mut used_rows = 0;
    for entry in entries.iter().skip(*scroll) {
        if used_rows + entry.rows.len() > content_height {
            break;
        }
        for (line, hit) in &entry.rows {
            push_boxed_line(out, line, inner);
            if let Some(hit) = hit {
                hits.push((top + 1 + used_rows, 0, cols, hit.clone()));
            }
            used_rows += 1;
        }
    }
    while used_rows < content_height {
        push_boxed_line(out, &" ".repeat(inner), inner);
        used_rows += 1;
    }
}

fn max_entry_scroll(entries: &[PanelEntry], content_height: usize) -> usize {
    let mut remaining: usize = entries.iter().map(|entry| entry.rows.len()).sum();
    let mut scroll = 0;
    while remaining > content_height && scroll < entries.len() {
        remaining = remaining.saturating_sub(entries[scroll].rows.len());
        scroll += 1;
    }
    scroll
}

fn push_boxed_line(out: &mut String, line: &str, inner: usize) {
    out.push('│');
    out.push_str(line);
    let used = graphemes::display_width(line);
    out.push_str(&" ".repeat(inner.saturating_sub(used)));
    out.push('│');
    out.push('\n');
}

/// Apply a muted charcoal surface across the full row, restoring it after
/// the segment-level resets emitted by `compose`.
fn selected_row(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 24);
    out.push_str(SELECTED_ROW_STYLE);
    let mut start = 0;
    while let Some(offset) = line[start..].find(RESET) {
        let end = start + offset + RESET.len();
        out.push_str(&line[start..end]);
        out.push_str(SELECTED_ROW_STYLE);
        start = end;
    }
    out.push_str(&line[start..]);
    out.push_str(RESET);
    out
}

fn entry_identity(entry: &crate::model::Entry) -> String {
    entry
        .app
        .clone()
        .or_else(|| {
            let title = entry.title.trim();
            (!title.is_empty()).then(|| title.to_string())
        })
        .unwrap_or_else(|| "pane".to_string())
}

fn agent_context(entry: &crate::model::Entry, include_pane_id: bool, inner: usize) -> String {
    let max_context = inner.saturating_sub(3); // indent, state glyph, space
    let mut session = entry.session.clone();
    let mut tab = tab_context(&entry.tab_name);
    let pane = if include_pane_id {
        let pane_id = match entry.pane_id {
            PaneId::Terminal(id) => id,
            PaneId::Plugin(id) => id,
        };
        Some(format!("p{pane_id}"))
    } else {
        None
    };

    let context_width = |session: &str, tab: &Option<String>, pane: &Option<String>| {
        let mut parts = Vec::new();
        if !session.is_empty() {
            parts.push(session.to_string());
        }
        if let Some(tab) = tab {
            parts.push(tab.clone());
        }
        if let Some(pane) = pane {
            parts.push(pane.clone());
        }
        graphemes::display_width(&parts.join(" · "))
    };

    // Preserve session context and the duplicate tag. A long tab name is the
    // first thing to shorten when the dim location row runs out of room.
    if context_width(&session, &tab, &pane) > max_context && pane.is_some() {
        tab = None;
    }
    if context_width(&session, &tab, &pane) > max_context {
        if let Some(tab_text) = tab.as_mut() {
            let session_width = if session.is_empty() {
                0
            } else {
                graphemes::display_width(&session) + 3
            };
            let pane_width = pane
                .as_ref()
                .map(|pane| 3 + graphemes::display_width(pane))
                .unwrap_or(0);
            let budget = max_context.saturating_sub(session_width + pane_width);
            if budget == 0 {
                tab = None;
            } else {
                *tab_text = graphemes::truncate_end(tab_text, budget, "…").into_owned();
            }
        }
    }
    if context_width(&session, &tab, &pane) > max_context && !session.is_empty() {
        let suffix_width = tab
            .as_ref()
            .map(|tab| 3 + graphemes::display_width(tab))
            .unwrap_or(0)
            + pane
                .as_ref()
                .map(|pane| 3 + graphemes::display_width(pane))
                .unwrap_or(0);
        session = graphemes::truncate_end(&session, max_context.saturating_sub(suffix_width), "…")
            .into_owned();
    }

    let mut parts = Vec::new();
    if !session.is_empty() {
        parts.push(session);
    }
    if let Some(tab) = tab {
        parts.push(tab);
    }
    if let Some(pane) = pane {
        parts.push(pane);
    }
    parts.join(" · ")
}

/// `╭─ title ───────────╮`, exactly `cols` wide.
fn boxed_top(cols: usize, title: &str) -> String {
    let title_w = graphemes::display_width(title);
    // overhead: "╭─ " (3) + " " (1) + "╮" (1) = 5
    let dots = cols.saturating_sub(title_w + 5);
    compose(
        vec![
            seg("╭─ "),
            styled(title, C_ACCENT),
            styled(format!(" {}╮", "─".repeat(dots)), DIM),
        ],
        cols,
    )
}

/// `╰─ hint ───────────╯`, exactly `cols` wide.
fn boxed_bottom(cols: usize, hint: &str) -> String {
    let budget = cols.saturating_sub(5);
    let hint_fit = if graphemes::display_width(hint) > budget {
        graphemes::truncate_end(hint, budget, "…").into_owned()
    } else {
        hint.to_string()
    };
    let fit_w = graphemes::display_width(&hint_fit);
    // overhead: "╰─ " (3) + " " + "╯" (1) = 5
    let dots = ((cols as isize) - ((fit_w + 5) as isize)).max(0) as usize;
    compose(
        vec![
            seg("╰─ "),
            styled(hint_fit, DIM),
            styled(format!(" {}╯", "─".repeat(dots)), DIM),
        ],
        cols,
    )
}

/// One body row.
struct Row {
    glyph: (&'static str, &'static str), // (char, color)
    title: String,
    title_style: &'static str,
    right: Vec<Seg>,
    hit: Option<Hit>,
}

impl Row {
    fn new(glyph: (&'static str, &'static str), title: impl Into<String>, hit: Option<Hit>) -> Row {
        Row {
            glyph,
            title: title.into(),
            title_style: "",
            right: Vec::new(),
            hit,
        }
    }

    fn style(mut self, style: &'static str) -> Row {
        self.title_style = style;
        self
    }

    fn right(mut self, text: impl Into<String>) -> Row {
        self.right = vec![styled(text, DIM)];
        self
    }

    fn left(&self) -> Vec<Seg> {
        vec![
            seg(" "),
            styled(self.glyph.0, self.glyph.1),
            seg(" "),
            styled(self.title.clone(), self.title_style),
        ]
    }
}

/// Compose a Row into a full-width line plus its hit.
fn row_line(row: Row, inner: usize) -> (String, Option<Hit>) {
    let hit = row.hit.clone();
    (compose_lr(row.left(), row.right, inner), hit)
}

fn section_header(label: &str, count: Option<usize>, inner: usize) -> String {
    let mut label_full = label.to_string();
    if let Some(count) = count {
        label_full.push_str(&format!(" ({count})"));
    }
    compose(vec![seg(" "), styled(label_full, BOLD)], inner)
}

/// What an entry's attention adds to its label, e.g. "permission?".
fn attention_hint(attention: &PaneAttention) -> String {
    let detail = &attention.detail;
    match &attention.status {
        AttentionStatus::NeedsYou => match detail.kind {
            Some(BlockedKind::Permission) => "permission?".to_string(),
            Some(BlockedKind::Question) => "question?".to_string(),
            Some(BlockedKind::Auth) => "login?".to_string(),
            None => "needs you".to_string(),
        },
        AttentionStatus::Failed => detail
            .exit_code
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "failed".to_string()),
        AttentionStatus::Working => match detail.progress {
            Some(p) => format!("{p}%"),
            None => String::new(),
        },
        _ => String::new(),
    }
}

fn counts_segments(c: &AttentionCounts) -> Vec<Seg> {
    let mut parts: Vec<Seg> = Vec::new();
    if c.needs_you > 0 {
        parts.push(styled(format!("◉ {} need", c.needs_you), C_NEEDS));
    }
    if c.failed > 0 {
        parts.push(styled(format!("✗ {} fail", c.failed), C_FAILED));
    }
    if c.working > 0 {
        parts.push(styled(format!("▸ {} run", c.working), C_WORK));
    }
    if c.done > 0 {
        parts.push(styled(format!("✓ {} done", c.done), C_DONE));
    }
    if c.new_output > 0 {
        parts.push(styled(format!("✚ {}", c.new_output), C_NEW_OUT));
    }
    if parts.is_empty() {
        return vec![styled("all quiet", DIM)];
    }
    let mut out: Vec<Seg> = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(styled(" · ", ""));
        }
        out.push(part);
    }
    out
}

fn session_counts_right(c: &AttentionCounts) -> String {
    let mut parts: Vec<String> = Vec::new();
    if c.needs_you > 0 {
        parts.push(format!("{} need", c.needs_you));
    }
    if c.failed > 0 {
        parts.push(format!("{} fail", c.failed));
    }
    if c.working > 0 {
        parts.push(format!("{} run", c.working));
    }
    if c.done > 0 {
        parts.push(format!("{} done", c.done));
    }
    parts.join(" · ")
}
