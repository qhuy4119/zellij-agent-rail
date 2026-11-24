//! Regression tests for the attention rail.
//!
//! These run on the host target (`cargo test`): zellij-tile's shim is a
//! no-op outside wasm32, so state and rendering can be exercised directly.
//!
//! Locked-down behaviors (each traces back to a reviewed bug):
//! - hit rows map to the *rendered* pane rows (the P0 offset bug)
//! - the render fills exactly `rows` lines of exactly `cols` width
//! - at the narrow fit boundary, context is dropped before the identity is
//!   truncated, and a kept context always has a gap before it
//! - completed agents are remembered; a reporting agent keeps its identity
//!   in later snapshots
//! - the AGENTS count only counts known agents

use zellij_tile::prelude::*;

use crate::model::{Entry, Hit, Rail};
use crate::render;

// ---------------------------------------------------------------- fixtures

fn attention(
    status: AttentionStatus,
    source: Option<AttentionSource>,
    app: Option<&str>,
    text: &str,
) -> PaneAttention {
    let mut attention = PaneAttention::default();
    attention.status = status;
    attention.source = source;
    attention.text = text.to_string();
    if let Some(app) = app {
        attention.detail.app = Some(app.to_string());
    }
    attention
}

fn remembered_done() -> PaneAttention {
    let mut attention = PaneAttention::default();
    attention.status = AttentionStatus::Idle;
    attention.remembered = Some(RememberedStatus {
        status: AttentionStatus::Done,
        text: String::new(),
        since_ms: 1,
    });
    attention
}

fn item(pane: u32, tab: &str, title: &str, attention: PaneAttention) -> (String, AttentionItem) {
    let it = AttentionItem {
        pane_id: PaneId::Terminal(pane),
        tab_id: 1,
        tab_name: tab.to_string(),
        pane_title: title.to_string(),
        attention,
    };
    ("main".to_string(), it)
}

fn session_attention() -> SessionAttention {
    SessionAttention::default()
}

fn global(items: Vec<(String, AttentionItem)>) -> GlobalAttention {
    let mut g = GlobalAttention::default();
    g.current_session = "main".to_string();
    g.sessions = vec![GlobalSessionAttention {
        name: "main".to_string(),
        is_current: true,
        attention: session_attention(),
    }];
    g.items = items
        .into_iter()
        .map(|(session, item)| {
            let is_current = session == "main";
            GlobalAttentionItem {
                session,
                is_current,
                item,
            }
        })
        .collect();
    g
}

fn rail_with(items: Vec<(String, AttentionItem)>) -> Rail {
    let mut rail = Rail::default();
    rail.set_global_attention(global(items));
    rail
}

/// Strip SGR/escape sequences for width assertions.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn display_width(s: &str) -> usize {
    graphemes::display_width(&strip_ansi(s))
}

fn rendered_lines(out: &str) -> Vec<&str> {
    out.split('\n').collect()
}

// ------------------------------------------------------------------- tests

/// The render must fill exactly `rows` lines, each exactly `cols` wide, with
/// the box closing at the bottom (regression: the trailing-newline bug that
/// scrolled the header away, and the floating-card layout).
#[test]
fn render_fills_exact_height_and_width() {
    let mut rail = rail_with(vec![item(
        1,
        "Tab #1",
        "π",
        attention(
            AttentionStatus::Working,
            Some(AttentionSource::ProgramStatus),
            Some("pi"),
            "refactoring",
        ),
    )]);
    let out = render::render(&mut rail, 39, 30);
    let lines = rendered_lines(&out);
    assert_eq!(lines.len(), 39, "must render exactly `rows` lines");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            display_width(line),
            30,
            "line {i} must be exactly `cols` wide: {line:?}"
        );
    }
    assert!(
        strip_ansi(lines[0]).starts_with("╭─ zellij"),
        "header on the first row"
    );
    assert!(
        strip_ansi(lines[38]).starts_with("╰─"),
        "footer on the last row"
    );
}

/// A click on the row where an agent renders must resolve to that agent's
/// hit (regression: hits stored body-relative, clicks pane-relative).
#[test]
fn hit_rows_match_rendered_rows() {
    let mut rail = rail_with(vec![
        item(
            1,
            "Tab #1",
            "t1",
            attention(
                AttentionStatus::Working,
                Some(AttentionSource::ProgramStatus),
                Some("pi"),
                "",
            ),
        ),
        item(
            2,
            "Tab #1",
            "t2",
            attention(
                AttentionStatus::NeedsYou,
                Some(AttentionSource::ProgramStatus),
                Some("claude"),
                "",
            ),
        ),
    ]);
    let out = render::render(&mut rail, 39, 30);
    let lines = rendered_lines(&out);

    // Find the rendered row of each agent identity.
    let pi_row = lines
        .iter()
        .position(|l| strip_ansi(l).contains("pi"))
        .expect("pi row rendered");
    let claude_row = lines
        .iter()
        .position(|l| strip_ansi(l).contains("claude"))
        .expect("claude row rendered");

    // Both rows must carry the matching hit at exactly that row.
    let hit_pane = |row: usize| -> Option<PaneId> {
        rail.hits
            .iter()
            .find(|(y, _, _, _)| *y == row)
            .and_then(|(_, _, _, hit)| match hit {
                Hit::Item { pane_id, .. } => Some(*pane_id),
                _ => None,
            })
    };
    assert_eq!(hit_pane(pi_row), Some(PaneId::Terminal(1)));
    assert_eq!(hit_pane(claude_row), Some(PaneId::Terminal(2)));

    // No hit may be registered on the chrome rows (header, counts, spacer)
    // or the footer.
    for (y, _, _, _) in &rail.hits {
        assert!(*y >= 3, "hit on chrome row {y}");
        assert!(*y < 38, "hit on footer row {y}");
    }

    // Clicking the rendered row resolves through the mouse path too.
    let mut rail2 = rail_with(vec![
        item(
            1,
            "Tab #1",
            "t1",
            attention(
                AttentionStatus::Working,
                Some(AttentionSource::ProgramStatus),
                Some("pi"),
                "",
            ),
        ),
        item(
            2,
            "Tab #1",
            "t2",
            attention(
                AttentionStatus::NeedsYou,
                Some(AttentionSource::ProgramStatus),
                Some("claude"),
                "",
            ),
        ),
    ]);
    let out2 = render::render(&mut rail2, 39, 30);
    let lines2 = rendered_lines(&out2);
    let claude_row2 = lines2
        .iter()
        .position(|l| strip_ansi(l).contains("claude"))
        .unwrap();
    let handled = rail2.handle_mouse(Mouse::LeftClick(claude_row2 as isize, 5));
    assert!(handled, "click on the claude row must act");
}

/// Long pane context must yield to the agent identity in the Herdr-style
/// two-line row. Duplicate panes retain a pane tag in context when it cannot
/// fit beside the agent identity.
#[test]
fn narrow_context_preserves_two_line_identity() {
    let long_identity = "a".repeat(22);
    let long_tab = "a-very-long-tab-name-that-should-drop";
    let mut rail = rail_with(vec![
        item(
            1,
            long_tab,
            "t1",
            attention(
                AttentionStatus::Done,
                Some(AttentionSource::ProgramStatus),
                Some(&long_identity),
                "",
            ),
        ),
        item(
            2,
            long_tab,
            "t2",
            attention(
                AttentionStatus::Done,
                Some(AttentionSource::ProgramStatus),
                Some(&long_identity),
                "",
            ),
        ),
    ]);
    let out = render::render(&mut rail, 39, 29); // inner = 27
    let plain: Vec<String> = rendered_lines(&out).into_iter().map(strip_ansi).collect();

    let label_rows: Vec<usize> = plain
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(&long_identity))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        label_rows.len(),
        2,
        "both duplicate identities render fully"
    );
    for (index, label_row) in label_rows.iter().enumerate() {
        assert!(plain[*label_row].contains(&long_identity));
        let context = &plain[label_row - 1];
        assert!(
            context.contains("main"),
            "session context retained: {context}"
        );
        assert!(
            context.contains(&format!("p{}", index + 1)),
            "pane id retained: {context}"
        );
        assert!(
            !context.contains("very-long-tab"),
            "low-priority tab dropped: {context}"
        );
        assert!(
            context.contains(" · p"),
            "context pieces are visibly separated: {context}"
        );
    }
}

#[test]
fn current_session_rows_are_highlighted() {
    let mut rail = rail_with(vec![]);
    rail.sessions[0].attention.counts.working = 1;
    rail.sessions.push(GlobalSessionAttention {
        name: "other".to_string(),
        is_current: false,
        attention: SessionAttention::default(),
    });

    let out = render::render(&mut rail, 39, 30);
    let raw_lines = rendered_lines(&out);
    let plain: Vec<String> = raw_lines.iter().map(|line| strip_ansi(line)).collect();
    let current = plain
        .iter()
        .position(|line| line.contains("main") && line.contains("current"))
        .expect("current session row rendered");
    assert!(
        raw_lines[current].contains("\u{1b}[48;2;38;38;44m"),
        "current row uses the muted charcoal selection surface"
    );
    assert!(
        !raw_lines[current].contains("\u{1b}[7m"),
        "selection does not invert foreground and background"
    );
    assert!(
        raw_lines[current + 1].contains("\u{1b}[48;2;38;38;44m"),
        "current session's secondary row shares the highlight"
    );

    let other = plain
        .iter()
        .position(|line| line.contains("other"))
        .expect("other session row rendered");
    assert!(
        !raw_lines[other].contains("\u{1b}[48;2;38;38;44m"),
        "other session is not selected"
    );
}

#[test]
fn sessions_stay_top_and_agents_anchor_low() {
    let mut rail = rail_with(vec![item(
        1,
        "api",
        "π",
        attention(
            AttentionStatus::Working,
            Some(AttentionSource::ProgramStatus),
            Some("pi"),
            "refactoring parser",
        ),
    )]);
    let out = render::render(&mut rail, 39, 30);
    let raw_lines = rendered_lines(&out);
    let lines: Vec<String> = raw_lines.iter().map(|line| strip_ansi(line)).collect();
    let sessions = lines
        .iter()
        .position(|line| line.contains("Sessions"))
        .unwrap();
    let agents = lines
        .iter()
        .position(|line| line.contains("Agents ("))
        .unwrap();
    assert!(
        sessions < 10,
        "sessions panel remains near the top: {sessions}"
    );
    assert!(
        agents > 18,
        "agents panel begins in the lower section: {agents}"
    );
    assert_eq!(agents, rail.agents_panel_start);

    let context = lines
        .iter()
        .position(|line| line.contains("main · api"))
        .expect("agent context row rendered");
    let label = &lines[context + 1];
    assert!(
        label.contains("pi · refactoring parser"),
        "agent label row: {label}"
    );
    assert!(
        lines[context].contains("▸"),
        "status icon belongs to context row"
    );
    assert!(
        raw_lines[context].contains("\u{1b}[2m"),
        "context row is dim"
    );
    assert!(
        raw_lines[context + 1].contains("\u{1b}[1m"),
        "agent label row is bold"
    );
}

#[test]
fn scroll_wheel_targets_the_hovered_panel() {
    let mut rail = rail_with(vec![item(
        1,
        "api",
        "π",
        attention(
            AttentionStatus::Working,
            Some(AttentionSource::ProgramStatus),
            Some("pi"),
            "",
        ),
    )]);
    render::render(&mut rail, 39, 30);

    let agents_top = rail.agents_panel_start as isize;
    rail.handle_mouse(Mouse::Hover(agents_top, 0));
    rail.handle_mouse(Mouse::ScrollDown(1));
    assert_eq!(rail.agents_scroll, 1);
    assert_eq!(rail.sessions_scroll, 0);

    rail.handle_mouse(Mouse::Hover(3, 0));
    rail.handle_mouse(Mouse::ScrollDown(1));
    assert_eq!(rail.sessions_scroll, 1);
    assert_eq!(rail.agents_scroll, 1);
}

/// A completed pane keeps its remembered done state and stays listed; an
/// agent that stops reporting keeps its identity through the cache.
#[test]
fn completions_and_identities_survive_snapshots() {
    let mut rail = rail_with(vec![
        // Reporting agent.
        item(
            1,
            "Tab #1",
            "t1",
            attention(
                AttentionStatus::Working,
                Some(AttentionSource::ProgramStatus),
                Some("pi"),
                "",
            ),
        ),
        // Completed pane: idle now, done remembered, never reported while
        // this instance ran.
        item(2, "Tab #1", "t2", remembered_done()),
    ]);
    let entries = rail.sidebar_entries();
    assert_eq!(entries.len(), 2, "both the agent and the completion listed");
    let done = entries
        .iter()
        .find(|e| e.pane_id == PaneId::Terminal(2))
        .expect("remembered completion retained");
    assert_eq!(
        done.status,
        AttentionStatus::Done,
        "remembered status shown"
    );
    assert!(!done.is_agent, "no invented identity for it");

    // Next snapshot: the agent stopped reporting (idle, no source, no
    // remembered state) but its pane still exists — the identity cache
    // must keep it an agent.
    rail.set_global_attention(global(vec![
        item(1, "Tab #1", "t1", PaneAttention::default()),
        item(2, "Tab #1", "t2", remembered_done()),
    ]));
    let entries = rail.sidebar_entries();
    let pi = entries
        .iter()
        .find(|e| e.pane_id == PaneId::Terminal(1))
        .expect("pane 1 still listed (agent via cache)");
    assert!(pi.is_agent, "identity cache keeps it an agent");
    assert_eq!(pi.app.as_deref(), Some("pi"));

    // A pane that closed (absent from the snapshot) drops from the cache.
    rail.set_global_attention(global(vec![item(2, "Tab #1", "t2", remembered_done())]));
    let entries = rail.sidebar_entries();
    assert!(
        !entries.iter().any(|e| e.pane_id == PaneId::Terminal(1)),
        "closed pane no longer listed"
    );
}

/// Idle agents are missing from GlobalAttentionUpdate, so PaneUpdate must
/// supply the current pane inventory and its retained OSC program status.
#[test]
fn idle_agents_from_pane_updates_stay_visible() {
    let mut rail = rail_with(vec![]);
    rail.set_tabs(vec![TabInfo {
        position: 0,
        tab_id: 1,
        name: "Tab #1".to_string(),
        ..TabInfo::default()
    }]);
    let mut idle_agent = PaneAttention::default();
    idle_agent.source = Some(AttentionSource::ProgramStatus);
    idle_agent.detail.app = Some("pi".to_string());
    let pane = PaneInfo {
        id: 7,
        title: "terminal".to_string(),
        attention: idle_agent,
        ..PaneInfo::default()
    };
    rail.set_panes(PaneManifest {
        panes: [(0, vec![pane])].into_iter().collect(),
    });

    let entries = rail.sidebar_entries();
    assert_eq!(
        entries.len(),
        1,
        "idle agent appears without an attention item"
    );
    assert!(entries[0].is_agent);
    assert_eq!(entries[0].app.as_deref(), Some("pi"));
    assert_eq!(entries[0].status, AttentionStatus::Idle);
    assert_eq!(entries[0].tab_name, "Tab #1");

    let out = render::render(&mut rail, 39, 30);
    let plain = strip_ansi(&out);
    assert!(plain.contains("Agents (1)"));
    assert!(plain.contains("pi"));

    // A later PaneUpdate is authoritative for pane liveness.
    rail.set_panes(PaneManifest::default());
    assert!(
        rail.sidebar_entries().is_empty(),
        "closed panes leave the rail"
    );
}

/// The AGENTS count counts known agents; urgent non-agent panes join the
/// list without inflating the count.
#[test]
fn agent_count_is_honest() {
    let mut rail = rail_with(vec![
        item(
            1,
            "Tab #1",
            "t1",
            attention(
                AttentionStatus::Working,
                Some(AttentionSource::ProgramStatus),
                Some("pi"),
                "",
            ),
        ),
        item(
            2,
            "Tab #1",
            "t2",
            attention(
                AttentionStatus::Done,
                Some(AttentionSource::ProgramStatus),
                Some("pi"),
                "",
            ),
        ),
        // A non-agent failed command pane.
        item(
            3,
            "Tab #1",
            "cargo build",
            attention(AttentionStatus::Failed, None, None, ""),
        ),
    ]);
    let entries = rail.sidebar_entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries.iter().filter(|e: &&Entry| e.is_agent).count(), 2);

    let out = render::render(&mut rail, 39, 30);
    let plain: Vec<String> = rendered_lines(&out)
        .into_iter()
        .map(|l| strip_ansi(l))
        .collect();
    assert!(
        plain.iter().any(|l| l.contains("Agents (2)")),
        "count reflects known agents only"
    );
    let heading = plain
        .iter()
        .find(|line| line.contains("Agents (2)"))
        .unwrap();
    assert!(
        !heading.contains('─'),
        "heading has no trailing rule: {heading}"
    );
}

/// A blocked agent that finished an earlier task must show its live
/// needs-you state, not the stale remembered completion (the reported bug).
#[test]
fn blocked_after_done_shows_live_state() {
    let mut blocked_after_done = PaneAttention::default();
    blocked_after_done.status = AttentionStatus::NeedsYou;
    blocked_after_done.source = Some(AttentionSource::ProgramStatus);
    blocked_after_done.detail.app = Some("claude".to_string());
    blocked_after_done.remembered = Some(RememberedStatus {
        status: AttentionStatus::Done,
        text: String::new(),
        since_ms: 1,
    });

    let rail = rail_with(vec![item(1, "Tab #1", "t1", blocked_after_done)]);
    let entries = rail.sidebar_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].status,
        AttentionStatus::NeedsYou,
        "live blocked state must win over the remembered completion"
    );

    // Working after a remembered done is the same rule.
    let mut working_after_done = PaneAttention::default();
    working_after_done.status = AttentionStatus::Working;
    working_after_done.source = Some(AttentionSource::ProgramStatus);
    working_after_done.detail.app = Some("claude".to_string());
    working_after_done.remembered = Some(RememberedStatus {
        status: AttentionStatus::Done,
        text: String::new(),
        since_ms: 1,
    });
    let rail = rail_with(vec![item(1, "Tab #1", "t1", working_after_done)]);
    assert_eq!(rail.sidebar_entries()[0].status, AttentionStatus::Working);
}

/// A remembered blocked record (e.g. a blocked subagent while the root task
/// keeps working) outranks the live working state.
#[test]
fn remembered_blocked_subagent_wins_over_working_root() {
    let mut attention = PaneAttention::default();
    attention.status = AttentionStatus::Working;
    attention.source = Some(AttentionSource::ProgramStatus);
    attention.detail.app = Some("claude".to_string());
    attention.remembered = Some(RememberedStatus {
        status: AttentionStatus::NeedsYou,
        text: String::new(),
        since_ms: 1,
    });
    let rail = rail_with(vec![item(1, "Tab #1", "t1", attention)]);
    assert_eq!(
        rail.sidebar_entries()[0].status,
        AttentionStatus::NeedsYou,
        "a remembered blocked record outranks live working"
    );
}
