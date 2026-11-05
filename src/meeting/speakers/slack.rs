//! Slack huddles in their expanded or compact pop-out window.
//! Bottom-bar mode exposes no reliable participant name and stays unnamed.
//!
//! Initial published evidence, followed by a controlled solo huddle on
//! Slack 4.53.280 (2026-11-05) confirming container, cells and profile labels.
//! The active-speaker class still needs a native remote-voice check:
//! Slack is Electron; clients can request its web tree using
//! `AXManualAccessibility`. Nodes expose Chromium's
//! `AXDOMIdentifier` and `AXDOMClassList` (Pipit PR #140, kawaz/shhcast
//! 2026-09-30, both on macOS). The huddle lives in a `p-huddle_window_content`
//! container (Lipik, macOS). Each participant is a `p-huddle_peer_tile`
//! with a `huddle-grid-gridcell-[self_]…` identifier and a "View <Name>'s
//! profile" label, and the tile being heard holds a node with the class
//! `p-huddle_peer_tile__overlay--active_speaker`: read live on Windows
//! through UI Automation (anomalyco/hex #859), and on macOS by Pipit, which
//! measured that at most one tile holds it, it moves between tiles at once,
//! and it lets go about 1.5 s after the voice stops. That makes it a turn,
//! not a voice-activity flag: two people talking over each other show as one.
//!
//! Only roles, classes, and identifiers are read until the container is
//! found; text is read inside participant tiles only. Messages, the
//! composer, threads, and sidebars are never entered.

use super::ax::{AxNode, Budget, Call, NoCall, Reading, has_class};
use super::names;

pub const BUNDLE_ID: &str = "com.tinyspeck.slackmacgap";
const CONTAINER: &str = "p-huddle_window_content";
const TILE: &str = "p-huddle_peer_tile";
const ACTIVE: &str = "p-huddle_peer_tile__overlay--active_speaker";
const NAME_OVERLAY: &str = "p-huddle_peer_tile__name_overlay";
const UNMUTED: &str = "p-huddle_peer_tile__name_overlay--unmuted";
const MIC_OVERLAY: &str = "p-huddle_peer_tile__mic_overlay";
const BOTTOM_BAR: &str = "p-huddle_peer_tile--bottom_bar";
const CELL_PREFIX: &str = "huddle-grid-gridcell-";
const SELF_CELL_PREFIX: &str = "huddle-grid-gridcell-self_";
/// The offscreen node that describes a tile for screen readers.
const DESCRIPTION_NODE: &str = "a11y_huddle_peer_tile_description";
/// A tile whose session has not resolved yet.
const PENDING_SUFFIX: &str = "_pending";
/// Subtrees whose classes mention these hold conversations, never a huddle.
const SKIP_WORDS: &[&str] = &[
    "message",
    "composer",
    "thread",
    "conversation",
    "channel_sidebar",
    "inbox",
    "reply",
    "comment",
    "texty",
    "search",
];
const SKIP_ROLES: &[&str] = &[
    "AXColumn",
    "AXTextArea",
    "AXTextField",
    "AXMenu",
    "AXMenuBar",
    "AXScrollBar",
];
/// A tile deeper or larger than this is read as unknown, never in part.
const TILE_DEPTH: usize = 16;
const TILE_NODES: usize = 256;

fn skipped<N: AxNode>(node: &N) -> bool {
    node.role()
        .is_some_and(|role| SKIP_ROLES.contains(&role.as_str()))
        || node.classes().iter().any(|class| {
            let class = class.to_lowercase();
            SKIP_WORDS.iter().any(|word| class.contains(word))
        })
}

/// Revalidate the unique huddle region in current windows every poll;
/// a previously bound region can survive after its call stops or moves.
pub fn read<N: AxNode>(windows: Vec<N>, budget: &mut Budget) -> Result<Call<N>, NoCall> {
    super::ax::read_complete(windows, |windows| {
        let region = find_container(windows, budget)?;
        let reading = read_tiles(&region, budget)?;
        Ok(Call { region, reading })
    })
}

fn find_container<N: AxNode>(windows: Vec<N>, budget: &mut Budget) -> Result<N, NoCall> {
    let mut stack = windows
        .into_iter()
        .rev()
        .map(|window| (window, 0))
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    let mut bottom_bar = false;
    while let Some((node, depth)) = stack.pop() {
        if !budget.visit() {
            return Err(NoCall::Unknown);
        }
        if has_class(&node, CONTAINER) {
            found.push(node);
            continue;
        }
        bottom_bar |= has_class(&node, BOTTOM_BAR);
        // Conversations are left out on purpose; anything else unread could
        // hide a second huddle.
        if skipped(&node) {
            continue;
        }
        let children = budget
            .children_at(&node, depth, Budget::MAX_DEPTH)
            .ok_or(NoCall::Unknown)?;
        stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    match found.len() {
        0 if bottom_bar => Err(NoCall::Unknown),
        0 => Err(NoCall::Absent),
        1 => Ok(found.remove(0)),
        // A docked huddle and a huddle window at once, or a stale one.
        _ => Err(NoCall::Unknown),
    }
}

#[derive(Debug, Default)]
struct Tile {
    cell: Option<String>,
    profile: Option<String>,
    speaking: bool,
    muted: bool,
}

impl Tile {
    fn is_self(&self) -> bool {
        self.cell
            .as_deref()
            .is_some_and(|cell| cell.starts_with(SELF_CELL_PREFIX))
    }

    /// The participant's name from its English profile label. Overlay
    /// buttons expose audio/video state, not a reliable name.
    fn name(&self) -> Option<String> {
        self.profile.as_deref().and_then(names::clean)
    }
}

fn read_tiles<N: AxNode>(region: &N, budget: &mut Budget) -> Result<Reading, NoCall> {
    let mut tiles = Vec::new();
    let mut stack = vec![(region.clone(), 0, None::<String>)];
    while let Some((node, depth, cell)) = stack.pop() {
        if !budget.visit() {
            return Err(NoCall::Unknown);
        }
        // Chromium mirrors the grid cells under AXColumn as well as rows.
        // Reading the column copy repeats each tile, not another participant.
        if node.role().as_deref() == Some("AXColumn") {
            continue;
        }
        let cell = cell.or_else(|| cell_id(&node));
        if has_class(&node, TILE) {
            tiles.push(read_tile(&node, cell, budget)?);
            continue;
        }
        let children = budget
            .children_at(&node, depth, Budget::MAX_DEPTH)
            .ok_or(NoCall::Unknown)?;
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1, cell.clone()));
        }
    }
    if tiles.is_empty() {
        return Err(NoCall::Absent);
    }
    let mut speaking = Vec::new();
    for tile in tiles.iter().filter(|tile| tile.speaking && !tile.is_self()) {
        let resolved = tile
            .cell
            .as_deref()
            .is_some_and(|cell| !cell.ends_with(PENDING_SUFFIX));
        match tile.name() {
            Some(name) if resolved && !tile.muted => {
                if !speaking.contains(&name) {
                    speaking.push(name);
                }
            }
            _ => return Ok(Reading::Uncertain),
        }
    }
    Ok(match speaking.len() {
        0 => Reading::Silent,
        1 => Reading::Speaking(speaking.remove(0)),
        _ => Reading::Uncertain,
    })
}

fn cell_id<N: AxNode>(node: &N) -> Option<String> {
    node.dom_id()
        .filter(|id| id.starts_with(CELL_PREFIX) && !id.contains(DESCRIPTION_NODE))
}

fn read_tile<N: AxNode>(
    tile: &N,
    cell: Option<String>,
    budget: &mut Budget,
) -> Result<Tile, NoCall> {
    let mut found = Tile {
        cell,
        ..Tile::default()
    };
    let mut stack = vec![(tile.clone(), 0)];
    let mut visited = 0;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > TILE_NODES || !budget.visit() {
            return Err(NoCall::Unknown);
        }
        let classes = node.classes();
        let id = node.dom_id().unwrap_or_default();
        if classes.iter().any(|class| class == "offscreen") || id.contains(DESCRIPTION_NODE) {
            continue;
        }
        found.cell = found.cell.or_else(|| cell_id(&node));
        found.speaking |= classes.iter().any(|class| class == ACTIVE);
        found.muted |= classes.iter().any(|class| class == MIC_OVERLAY)
            || (classes.iter().any(|class| class == NAME_OVERLAY)
                && !classes.iter().any(|class| class == UNMUTED));
        if found.profile.is_none() {
            found.profile = node
                .description()
                .as_deref()
                .and_then(profile_name)
                .or_else(|| node.title().as_deref().and_then(profile_name));
        }
        // A mark below what was read could be on this tile.
        let children = budget
            .children_at(&node, depth, TILE_DEPTH)
            .ok_or(NoCall::Unknown)?;
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    Ok(found)
}

/// "View Priya's profile" gives "Priya"; any other label names nobody.
fn profile_name(label: &str) -> Option<String> {
    let rest = label.strip_prefix("View ")?;
    let name = rest
        .strip_suffix("'s profile")
        .or_else(|| rest.strip_suffix("\u{2019}s profile"))?;
    Some(name.trim().to_owned()).filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::ax::fake::{Builder, Node};
    use super::*;

    fn budget() -> Budget {
        Budget::new(10_000, Duration::from_secs(5))
    }

    fn tile(cell: &str, name: &str, speaking: bool) -> Builder {
        let profile = format!("View {name}'s profile");
        let mut tile = Node::element("AXCell")
            .class(TILE)
            .id(&format!("{CELL_PREFIX}{cell}"))
            .title(&profile)
            .description(&profile)
            .child(
                Node::element("AXButton")
                    .class(NAME_OVERLAY)
                    .class(UNMUTED)
                    .description("video is off, audio is on"),
            );
        if speaking {
            tile = tile.child(Node::element("AXGroup").class(ACTIVE));
        }
        tile
    }

    fn huddle(tiles: impl IntoIterator<Item = Builder>) -> Builder {
        Node::element("AXGroup").class(CONTAINER).child(
            Node::element("AXGroup")
                .class("p-huddle_peer_grid")
                .children(tiles),
        )
    }

    /// A Slack window with conversation content beside the huddle.
    fn window(huddle: Option<Builder>) -> Node {
        let messages = Node::element("AXGroup").class("c-message_list").child(
            Node::element("AXGroup")
                .class("c-message_kit__message")
                .child(
                    Node::element("AXStaticText").title("Grace is speaking: View Grace's profile"),
                )
                .child(Node::element("AXGroup").class(ACTIVE)),
        );
        let mut workspace = Node::element("AXGroup")
            .class("p-client_workspace")
            .child(messages);
        if let Some(huddle) = huddle {
            workspace = workspace.child(huddle);
        }
        Node::element("AXWindow")
            .title("general - Slack")
            .child(workspace)
            .build()
    }

    fn reading(windows: Vec<Node>) -> Result<Reading, NoCall> {
        read(windows, &mut budget()).map(|call| call.reading)
    }

    const REMOTE: &str = "a2c1e0_U01AAAAAAAA";
    const OTHER: &str = "b3d2f1_U01BBBBBBBB";
    const SELF: &str = "self_U01CCCCCCCC";

    #[test]
    fn the_marked_remote_tile_names_the_speaker_in_a_huddle_window() {
        let window = Node::element("AXWindow")
            .title("Huddle: #design - Slack")
            .child(huddle([
                tile(REMOTE, "Dev Patel", true),
                tile(OTHER, "Eli Novak", false),
                tile(SELF, "Fay Lin", false),
            ]))
            .build();
        assert_eq!(
            reading(vec![window]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn a_full_grid_container_reads_without_touching_adjacent_messages() {
        let main = window(Some(huddle([
            tile(REMOTE, "Dev Patel", false),
            tile(OTHER, "Eli Novak", true),
        ])));
        assert_eq!(
            reading(vec![main.clone()]),
            Ok(Reading::Speaking("Eli Novak".into()))
        );
        // Nothing outside the tiles was read as text: not the message, not
        // the window title.
        assert!(main.reads().iter().all(|text| !text.contains("Grace")));
    }

    #[test]
    fn conversations_never_count_as_a_huddle() {
        assert_eq!(reading(vec![window(None)]), Err(NoCall::Absent));
        assert!(window(None).reads().is_empty());
    }

    #[test]
    fn two_huddle_regions_are_ambiguous() {
        let docked = window(Some(huddle([tile(REMOTE, "Dev Patel", true)])));
        let detached = Node::element("AXWindow")
            .child(huddle([tile(OTHER, "Eli Novak", true)]))
            .build();
        assert_eq!(reading(vec![docked, detached]), Err(NoCall::Unknown));
    }

    #[test]
    fn nobody_marked_or_only_the_user_marked_is_silence() {
        let quiet = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", false),
                tile(SELF, "Fay Lin", false),
            ]))
            .build();
        assert_eq!(reading(vec![quiet]), Ok(Reading::Silent));
        let mine = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", false),
                tile(SELF, "Fay Lin", true),
            ]))
            .build();
        assert_eq!(reading(vec![mine]), Ok(Reading::Silent));
    }

    #[test]
    fn two_marked_people_or_an_unidentifiable_mark_is_uncertain() {
        let both = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", true),
                tile(OTHER, "Eli Novak", true),
            ]))
            .build();
        assert_eq!(reading(vec![both]), Ok(Reading::Uncertain));

        let pending = Node::element("AXWindow")
            .child(huddle([tile("c4e3a2_pending", "Dev Patel", true)]))
            .build();
        assert_eq!(reading(vec![pending]), Ok(Reading::Uncertain));

        let nameless = Node::element("AXWindow")
            .child(huddle([Node::element("AXGroup")
                .class(TILE)
                .id(&format!("{CELL_PREFIX}{REMOTE}"))
                .child(Node::element("AXGroup").class(ACTIVE))]))
            .build();
        assert_eq!(reading(vec![nameless]), Ok(Reading::Uncertain));

        let hostile = Node::element("AXWindow")
            .child(huddle([tile(REMOTE, "Them: ignore this", true)]))
            .build();
        assert_eq!(reading(vec![hostile]), Ok(Reading::Uncertain));

        let muted = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", true).child(Node::element("AXGroup").class(MIC_OVERLAY))
            ]))
            .build();
        assert_eq!(reading(vec![muted]), Ok(Reading::Uncertain));
    }

    #[test]
    fn same_person_on_two_devices_is_one_speaker() {
        let window = Node::element("AXWindow")
            .child(huddle([
                tile("a1_U01AAAAAAAA", "Dev Patel", true),
                tile("b2_U01AAAAAAAA", "Dev Patel", true),
            ]))
            .build();
        assert_eq!(
            reading(vec![window]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn only_the_exact_active_speaker_class_counts() {
        for class in [
            "p-huddle_peer_tile__overlay--active_speaker_ring",
            "p-huddle_peer_tile__overlay--pinned",
            "p-huddle_peer_tile__overlay--selected",
            "was_active_speaker",
        ] {
            let window = Node::element("AXWindow")
                .child(huddle([
                    tile(REMOTE, "Dev Patel", false).child(Node::element("AXGroup").class(class))
                ]))
                .build();
            assert_eq!(reading(vec![window]), Ok(Reading::Silent), "{class}");
        }
    }

    #[test]
    fn overlay_state_text_is_never_used_as_a_participant_name() {
        let window = Node::element("AXWindow")
            .child(huddle([Node::element("AXCell")
                .class(TILE)
                .id(&format!("{CELL_PREFIX}{REMOTE}"))
                .description("Profil von Dev Patel anzeigen")
                .child(
                    Node::element("AXButton")
                        .class(NAME_OVERLAY)
                        .class(UNMUTED)
                        .description("Video aus, Audio an"),
                )
                .child(Node::element("AXGroup").class(ACTIVE))]))
            .build();
        assert_eq!(reading(vec![window]), Ok(Reading::Uncertain));
    }

    #[test]
    fn compact_cells_need_no_name_overlay_but_require_a_profile_label() {
        let window = Node::element("AXWindow")
            .child(huddle([Node::element("AXCell")
                .class(TILE)
                .id(&format!("{CELL_PREFIX}{REMOTE}"))
                .title("View Dev Patel's profile")
                .child(Node::element("AXGroup").class(ACTIVE))]))
            .build();
        assert_eq!(
            reading(vec![window]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn a_muted_overlay_without_the_unmuted_modifier_rejects_a_lingering_mark() {
        let window = Node::element("AXWindow")
            .child(huddle([tile(REMOTE, "Dev Patel", true).child(
                Node::element("AXButton")
                    .class(NAME_OVERLAY)
                    .description("video is off, audio is off"),
            )]))
            .build();
        assert_eq!(reading(vec![window]), Ok(Reading::Uncertain));
    }

    #[test]
    fn bottom_bar_without_a_full_grid_is_unavailable_not_absent() {
        let bar = || Node::element("AXGroup").class(BOTTOM_BAR);
        assert_eq!(reading(vec![window(Some(bar()))]), Err(NoCall::Unknown));
        let expanded = Node::element("AXWindow")
            .child(huddle([tile(REMOTE, "Dev Patel", true)]))
            .build();
        assert_eq!(
            reading(vec![window(Some(bar())), expanded]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn duplicate_column_cells_are_not_read_again() {
        let window = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", true),
                Node::element("AXColumn")
                    .unreadable()
                    .child(tile(REMOTE, "Dev Patel", true)),
            ]))
            .build();
        assert_eq!(
            reading(vec![window]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn the_screen_reader_description_node_is_not_read_as_a_tile() {
        let window = Node::element("AXWindow")
            .child(huddle([tile(REMOTE, "Dev Patel", false).child(
                Node::element("AXGroup")
                    .class("offscreen")
                    .id(&format!("{CELL_PREFIX}{REMOTE}-{DESCRIPTION_NODE}"))
                    .child(Node::element("AXGroup").class(ACTIVE)),
            )]))
            .build();
        assert_eq!(reading(vec![window]), Ok(Reading::Silent));
    }

    #[test]
    fn every_poll_revalidates_the_unique_current_container() {
        let first = Node::element("AXWindow")
            .child(huddle([tile(REMOTE, "Dev Patel", true)]))
            .build();
        let second = Node::element("AXWindow")
            .child(huddle([tile(OTHER, "Eli Novak", true)]))
            .build();
        let old = read(vec![first.clone()], &mut budget()).unwrap().region;
        assert_eq!(
            reading(vec![first.clone(), second.clone()]),
            Err(NoCall::Unknown)
        );
        assert_eq!(reading(Vec::<Node>::new()), Err(NoCall::Absent));
        let current = read(vec![second], &mut budget()).unwrap();
        assert!(!current.region.same_element(&old));
        assert_eq!(current.reading, Reading::Speaking("Eli Novak".into()));
    }

    #[test]
    fn a_walk_that_runs_out_of_budget_is_unknown() {
        let window = window(Some(huddle([tile(REMOTE, "Dev Patel", true)])));
        let mut small = Budget::new(3, Duration::from_secs(5));
        assert_eq!(
            read(vec![window], &mut small).map(|call| call.reading),
            Err(NoCall::Unknown)
        );
        assert!(small.truncated());
    }

    #[test]
    fn a_failed_class_request_cannot_hide_a_second_speaker() {
        let window = Node::element("AXWindow")
            .child(huddle([
                tile(REMOTE, "Dev Patel", true),
                tile(OTHER, "Eli Novak", false)
                    .child(Node::element("AXGroup").class(ACTIVE).unreadable_classes()),
            ]))
            .build();
        assert_eq!(reading(vec![window]), Err(NoCall::Unknown));
    }

    #[test]
    fn anything_hex_could_not_read_makes_the_reading_unknown() {
        use super::super::ax::fake::{crowd, nested};
        let dev = || tile(REMOTE, "Dev Patel", true);
        let eli = || tile(OTHER, "Eli Novak", true);
        let window = |tiles: Vec<Builder>| Node::element("AXWindow").child(huddle(tiles)).build();
        // Eli's tile is deeper than the walk goes.
        assert_eq!(
            reading(vec![window(vec![dev(), nested(Budget::MAX_DEPTH, eli())])]),
            Err(NoCall::Unknown)
        );
        // The app fails to list the children holding Eli's tile.
        assert_eq!(
            reading(vec![window(vec![
                dev(),
                Node::element("AXGroup").unreadable().child(eli()),
            ])]),
            Err(NoCall::Unknown)
        );
        // More children than one element may report.
        assert_eq!(
            reading(vec![window(vec![
                dev(),
                Node::element("AXGroup").children(crowd(Budget::MAX_CHILDREN, eli())),
            ])]),
            Err(NoCall::Unknown)
        );
        // A mark deeper in a tile than the tile read goes.
        let deep_mark = Node::element("AXGroup")
            .class(TILE)
            .id(&format!("{CELL_PREFIX}{OTHER}"))
            .child(Node::element("AXButton").description("View Eli Novak's profile"))
            .child(nested(TILE_DEPTH, Node::element("AXGroup").class(ACTIVE)));
        assert_eq!(
            reading(vec![window(vec![dev(), deep_mark])]),
            Err(NoCall::Unknown)
        );
        // A huddle the container search could only partly read is unknown
        // too: a second container could be in the unread part.
        let hidden = Node::element("AXWindow")
            .child(huddle([dev()]))
            .child(Node::element("AXGroup").unreadable())
            .build();
        assert_eq!(reading(vec![hidden]), Err(NoCall::Unknown));
        // Conversations, which are skipped on purpose, never make it unknown.
        assert_eq!(
            reading(vec![window(vec![dev()])]),
            Ok(Reading::Speaking("Dev Patel".into()))
        );
    }

    #[test]
    fn an_empty_container_is_not_a_call() {
        let window = Node::element("AXWindow").child(huddle([])).build();
        assert_eq!(reading(vec![window]), Err(NoCall::Absent));
    }
}
